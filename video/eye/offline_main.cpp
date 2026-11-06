// Explicit raw-file harness: no OpenCV camera API, audio, UI, or network API.
// Container decode/encode is a separate local step. Outputs never overwrite.
#include "ar087_provider.h"
#include "session.h"
#include <windows.h>
#include <algorithm>
#include <iostream>
#include <string>
using namespace tingxian::eye;
namespace {
struct Handle {
    HANDLE value = INVALID_HANDLE_VALUE;
    ~Handle() { if (value != INVALID_HANDLE_VALUE) CloseHandle(value); }
};
struct MonotonicClock : Clock { uint64_t now_ms() const noexcept override { return GetTickCount64(); } };
struct FileSink : OutputSink {
    HANDLE file;
    std::vector<uint8_t> output;
    bool visible = false;
    explicit FileSink(HANDLE handle, size_t size) : file(handle), output(size, 32) {}
    void neutral(uint64_t) noexcept override { std::fill(output.begin(), output.end(), uint8_t{32}); visible = false; }
    bool corrected(const Frame& frame) noexcept override {
        if (frame.bgr.size() != output.size()) return false;
        std::copy(frame.bgr.begin(), frame.bgr.end(), output.begin()); visible = true; return true;
    }
    bool write() {
        DWORD count = 0;
        return WriteFile(file, output.data(), DWORD(output.size()), &count, nullptr) && count == output.size();
    }
};
unsigned number(const wchar_t* text) {
    if (!text || !*text) return 0;
    unsigned value = 0;
    for (; *text; ++text) {
        if (*text < L'0' || *text > L'9' || value > 1000000) return 0;
        value = value * 10 + unsigned(*text - L'0');
    }
    return value;
}
}
int wmain(int argc, wchar_t** argv) {
    if (argc != 7) return 2;
    const std::wstring mode(argv[1]);
    if (mode != L"--offline-bgr-v1" && mode != L"--offline-bgr-restart3-v1") return 2;
    const unsigned cycles = mode == L"--offline-bgr-restart3-v1" ? 3 : 1;
    Format format{number(argv[2]), number(argv[3])};
    const auto expected = number(argv[4]);
    if (!format.bytes() || !expected || expected > 100000) return 2;
    Handle input, output;
    input.value = CreateFileW(argv[5], GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
    if (input.value == INVALID_HANDLE_VALUE) return 3;
    BY_HANDLE_FILE_INFORMATION info{}; LARGE_INTEGER size{};
    if (!GetFileInformationByHandle(input.value, &info) || (info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT))
        || !GetFileSizeEx(input.value, &size) || uint64_t(size.QuadPart) != uint64_t(format.bytes()) * expected) return 3;
    output.value = CreateFileW(argv[6], GENERIC_WRITE, 0, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (output.value == INVALID_HANDLE_VALUE) return 4;
    auto provider = make_ar087_provider();
    MonotonicClock clock; FileSink sink(output.value, format.bytes()); Session session(*provider, sink, clock);
    uint64_t corrected = 0, neutral = 0; int exit_code = 0;
    unsigned completed_cycles = 0; Error error = Error::none;
    for (unsigned cycle = 0; cycle < cycles; ++cycle) {
      LARGE_INTEGER zero{};
      if (!SetFilePointerEx(input.value, zero, nullptr, FILE_BEGIN)) { exit_code = 6; break; }
      const auto generation = session.begin(format);
      if (!generation || !session.prepare(generation)) {
          std::cerr << "provider_prepare_failed\n"; error = session.snapshot().error;
          exit_code = 5; session.stop(); session.drain_stop(); break;
      }
      for (unsigned id = 1; id <= expected; ++id) {
        Frame frame{generation, id, clock.now_ms(), std::vector<uint8_t>(format.bytes())};
        DWORD count = 0;
        if (!ReadFile(input.value, frame.bgr.data(), DWORD(frame.bgr.size()), &count, nullptr) || count != frame.bgr.size()) { exit_code = 6; break; }
        frame.captured_ms = clock.now_ms();
        if (!session.submit(std::move(frame)) || !session.process_one()) {
            sink.neutral(session.snapshot().generation);
            ++neutral;
            if (!sink.write()) exit_code = 7;
            if (session.snapshot().state != State::ready) { if (!exit_code) exit_code = 8; break; }
        } else { ++corrected; if (!sink.write()) exit_code = 7; }
        if (exit_code) break;
      }
      error = session.snapshot().error;
      session.stop(); session.drain_stop();
      if (session.snapshot().cleanup_uncertain) exit_code = 9;
      if (exit_code) break;
      ++completed_cycles;
    }
    const auto snapshot = session.snapshot();
    if (session.snapshot().cleanup_uncertain) exit_code = 9;
    if (!FlushFileBuffers(output.value) && !exit_code) exit_code = 7;
    std::cout << "{\"corrected\":" << corrected << ",\"neutral\":" << neutral
        << ",\"error\":" << static_cast<int>(error) << ",\"dropped\":" << snapshot.dropped
        << ",\"completed_cycles\":" << completed_cycles
        << ",\"exit_code\":" << exit_code << ",\"camera_started\":false,\"audio_started\":false}\n";
    return exit_code;
}
