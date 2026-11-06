// Inherited-handle offline worker. No capture, device registration or network API.
#include "session.h"
#include "owner_watch.h"
#include "ar087_provider.h"
#include <windows.h>
#include <atomic>
#include <cerrno>
#include <memory>
#include <sstream>
#include <string>
#ifdef TINGXIAN_EYE_SYNTHETIC
#include "worker_synthetic.h"
#endif
using namespace tingxian::eye;
namespace {
struct Handles {
    HANDLE owner, stop, report, input, output;
    ~Handles() { for (auto h : {owner, stop, report, input, output}) if (h && h != INVALID_HANDLE_VALUE) CloseHandle(h); }
};
uint64_t number(const wchar_t* text) {
    if (!text || !*text) return 0;
    for (auto p = text; *p; ++p) if (*p < L'0' || *p > L'9') return 0;
    errno = 0; const auto value = _wcstoui64(text, nullptr, 10);
    return errno ? 0 : value;
}
struct MonotonicClock : Clock { uint64_t now_ms() const noexcept override { return GetTickCount64(); } };
// A file is diagnostic until the supervisor accepts the COMPLETE report. It is
// never a live sink. Immutable snapshots prevent the cancellation watcher racing
// with disk I/O; an in-flight disk write may finish into the rejected partial file.
struct FileSink : OutputSink {
    std::shared_ptr<const std::vector<uint8_t>> pending;
    void neutral(uint64_t) noexcept override { std::atomic_store(&pending, std::shared_ptr<const std::vector<uint8_t>>{}); }
    bool corrected(const Frame& frame) noexcept override {
        try {
            std::shared_ptr<const std::vector<uint8_t>> value = std::make_shared<std::vector<uint8_t>>(frame.bgr);
            std::atomic_store(&pending, std::move(value)); return true;
        } catch (...) { return false; }
    }
    std::shared_ptr<const std::vector<uint8_t>> take() { return std::atomic_exchange(&pending, std::shared_ptr<const std::vector<uint8_t>>{}); }
};
bool write_frame(HANDLE file, const std::vector<uint8_t>& bytes) {
    DWORD written = 0;
    return WriteFile(file, bytes.data(), DWORD(bytes.size()), &written, nullptr) && written == bytes.size();
}
bool disk_file(HANDLE file, uint64_t expected_size) {
    BY_HANDLE_FILE_INFORMATION info{}; LARGE_INTEGER size{};
    return GetFileType(file) == FILE_TYPE_DISK && GetFileInformationByHandle(file, &info)
        && !(info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT))
        && info.nNumberOfLinks == 1 && GetFileSizeEx(file, &size) && uint64_t(size.QuadPart) == expected_size;
}
}
int wmain(int argc, wchar_t** argv) {
    if (argc != 11 || std::wstring(argv[1]) != L"--eye-offline-v1") return 2;
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    if (!SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_APPLICATION_DIR)) return 3;
    const auto width = number(argv[7]), height = number(argv[8]), count = number(argv[9]), request = number(argv[10]);
    if (!width || !height || width > 1920 || height > 1080 || !count || count > 100000 || !request) return 2;
    Format format{uint32_t(width), uint32_t(height)};
    Handles handles{reinterpret_cast<HANDLE>(number(argv[2])), reinterpret_cast<HANDLE>(number(argv[3])),
        reinterpret_cast<HANDLE>(number(argv[4])), reinterpret_cast<HANDLE>(number(argv[5])), reinterpret_cast<HANDLE>(number(argv[6]))};
    if (!disk_file(handles.input, format.bytes() * count) || !disk_file(handles.output, 0)
        || GetFileType(handles.report) != FILE_TYPE_PIPE) return 3;
    LARGE_INTEGER zero{};
    if (!SetFilePointerEx(handles.input, zero, nullptr, FILE_BEGIN) || !SetFilePointerEx(handles.output, zero, nullptr, FILE_BEGIN)) return 3;
#ifdef TINGXIAN_EYE_SYNTHETIC
    auto provider = make_synthetic_worker_provider(handles.stop);
    constexpr const char* provider_name = "synthetic";
#else
    auto provider = make_ar087_provider();
    constexpr const char* provider_name = "ar087";
#endif
    FileSink sink; MonotonicClock clock; Session session(*provider, sink, clock);
    const auto generation = session.begin(format); // capture BEFORE the watcher
    OwnerWatch watch(session);
    if (!generation || !watch.arm(handles.owner, handles.stop)) return 3;
    uint32_t error = 0, corrected = 0, neutral = 0;
    if (!session.prepare(generation)) error = uint32_t(session.snapshot().error);
    else for (uint32_t id = 1; id <= count && !watch.cancelled(); ++id) {
        Frame frame{generation, id, clock.now_ms(), std::vector<uint8_t>(format.bytes())};
        DWORD read = 0;
        if (!ReadFile(handles.input, frame.bgr.data(), DWORD(frame.bgr.size()), &read, nullptr) || read != frame.bgr.size()) { error = 9; break; }
        frame.captured_ms = clock.now_ms();
        const bool processed = session.submit(std::move(frame)) && session.process_one();
        if (watch.cancelled()) break;
        if (!processed) {
            error = uint32_t(session.snapshot().error);
            if (!error) error = 9; // expired/invalid offline frame is not completion
            if (write_frame(handles.output, std::vector<uint8_t>(format.bytes(), 32))) ++neutral;
            else error = 10;
            break;
        }
        auto value = sink.take();
        if (watch.cancelled()) break;
        if (!value || !write_frame(handles.output, *value)) { error = 10; break; }
        ++corrected;
    }
    const bool had_cleanup_failure = session.snapshot().cleanup_uncertain;
    session.stop(); session.drain_stop();
    watch.close(); // joins the watcher before evaluating cancellation/report
    const bool uncertain = had_cleanup_failure || session.snapshot().cleanup_uncertain;
    if (uncertain) error = 8;
    const bool cancelled = watch.cancelled() && !uncertain;
    if (!FlushFileBuffers(handles.output) && !uncertain) error = 10;
    const char* outcome = cancelled ? "cancelled" : error ? "fault" : "completed";
    std::ostringstream json;
    json << "{\"protocol\":2,\"generation\":" << request << ",\"provider\":\"" << provider_name
        << "\",\"width\":" << width << ",\"height\":" << height << ",\"expected_frames\":" << count
        << ",\"corrected\":" << corrected << ",\"neutral\":" << neutral << ",\"outcome\":\"" << outcome
        << "\",\"error\":" << error << ",\"cleanup_uncertain\":" << (uncertain ? "true" : "false")
        << ",\"camera_started\":false,\"audio_started\":false,\"external_output_started\":false}";
    const auto text = json.str(); DWORD written = 0;
    return WriteFile(handles.report, text.data(), DWORD(text.size()), &written, nullptr) && written == text.size() ? 0 : 4;
}
