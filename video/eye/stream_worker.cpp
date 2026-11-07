#include "stream_protocol.h"
#include "session.h"
#include "owner_watch.h"
#include "ar087_provider.h"
#include "../windows/eye_output.h"
#include <algorithm>
#include <cerrno>
#include <memory>
#include <sstream>
#ifdef TINGXIAN_EYE_SYNTHETIC
#include "worker_synthetic.h"
#endif

namespace tingxian::eye::stream {
namespace {
struct Handles {
    HANDLE owner, stop, report, input;
    ~Handles() { for (auto h : {owner, stop, report, input}) if (h && h != INVALID_HANDLE_VALUE) CloseHandle(h); }
};
uint64_t number(const wchar_t* value) {
    if (!value || !*value) return 0;
    for (auto p = value; *p; ++p) if (*p < L'0' || *p > L'9') return 0;
    errno = 0; const auto n = _wcstoui64(value, nullptr, 10); return errno ? 0 : n;
}
struct Monotonic : Clock { uint64_t now_ms() const noexcept override { return GetTickCount64(); } };
enum class Read { complete, cancelled, input_fault, output_fault };
bool failed_output(const video::EyePipeOutput& sink) {
    const auto state = sink.status().state;
    return state == video::EyeOutputState::fault || state == video::EyeOutputState::stopped;
}
// Exactly one reader owns the anonymous input pipe. Read only bytes already
// available, with an absolute deadline (not renewed by a slow partial writer).
// OwnerWatch independently revokes output. The host Job remains the backstop
// for a vendor/OS call that fails to return; this is not a hard realtime API.
Read read_exact(HANDLE pipe, void* target, DWORD bytes, DWORD timeout,
                OwnerWatch& watch, const video::EyePipeOutput& sink) {
    auto data = static_cast<BYTE*>(target); DWORD done = 0;
    const auto deadline = GetTickCount64() + timeout;
    while (done < bytes) {
        if (watch.cancelled()) return Read::cancelled;
        if (failed_output(sink)) return Read::output_fault;
        if (GetTickCount64() >= deadline) return Read::input_fault;
        DWORD available = 0, count = 0;
        if (!PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr)) return Read::input_fault;
        if (!available) { Sleep(2); continue; }
        if (!ReadFile(pipe, data + done, (std::min)(available, bytes - done), &count, nullptr) || !count) return Read::input_fault;
        done += count;
    }
    return Read::complete;
}
bool write_small(HANDLE pipe, const void* data, DWORD bytes) {
    DWORD written = 0;
    return WriteFile(pipe, data, bytes, &written, nullptr) && written == bytes;
}
}
bool valid_input(const InputHeader& h, uint64_t generation) noexcept {
    if (h.magic != InputMagic || h.version != 1 || h.reserved || h.generation != generation) return false;
    if (h.kind == Finish) return h.bytes == 0 && h.id == 0 && h.captured_ms == 0;
    return h.kind == Frame && h.bytes == BgrBytes && h.id != 0;
}
int run(int argc, wchar_t** argv) {
    // owner, stop, report, input, fixed width/height, host generation
    if (argc != 9 || number(argv[6]) != video::Width || number(argv[7]) != video::Height || !number(argv[8])) return 2;
    const auto request = number(argv[8]);
    Handles handles{reinterpret_cast<HANDLE>(number(argv[2])), reinterpret_cast<HANDLE>(number(argv[3])),
                    reinterpret_cast<HANDLE>(number(argv[4])), reinterpret_cast<HANDLE>(number(argv[5]))};
    if (GetFileType(handles.input) != FILE_TYPE_PIPE || GetFileType(handles.report) != FILE_TYPE_PIPE) return 3;
#ifdef TINGXIAN_EYE_SYNTHETIC
    auto provider = make_synthetic_worker_provider(handles.stop);
    constexpr const char* provider_name = "synthetic";
#else
    auto provider = make_ar087_provider();
    constexpr const char* provider_name = "ar087";
#endif
    video::EyePipeOutput sink; Monotonic clock; Session session(*provider, sink, clock);
    const auto generation = session.begin({video::Width, video::Height});
    const auto ticket = sink.operation_ticket(); // AFTER begin neutralizes, BEFORE owner watch
    OwnerWatch watch(session);
    if (!generation || !watch.arm(handles.owner, handles.stop)) return 3;
    uint32_t error = 0; uint64_t received = 0; bool finished = false, announced = false;
    try {
        if (!session.prepare(generation)) error = uint32_t(session.snapshot().error);
        else if (!watch.cancelled()) {
            Ready ready{}; ready.header.generation = request;
            if (FAILED(sink.open_if_current(generation, ticket, &ready.endpoint))) error = 10;
            else if (!watch.cancelled()) announced = write_small(handles.report, &ready, ReadyBytes);
            SecureZeroMemory(&ready, sizeof(ready));
            if (!announced && !watch.cancelled()) error = 10;
        }
        while (!error && announced && !watch.cancelled()) {
            InputHeader header{};
            auto read = read_exact(handles.input, &header, sizeof(header), IdleTimeoutMs, watch, sink);
            if (read != Read::complete) { if (read != Read::cancelled) error = read == Read::output_fault ? 10 : 9; break; }
            if (!valid_input(header, request)) { error = 9; break; }
            if (header.kind == Finish) { finished = true; break; }
            eye::Frame frame{generation, header.id, header.captured_ms, std::vector<uint8_t>(BgrBytes)};
            read = read_exact(handles.input, frame.bgr.data(), BgrBytes, PayloadTimeoutMs, watch, sink);
            if (read != Read::complete) { if (read != Read::cancelled) error = read == Read::output_fault ? 10 : 9; break; }
            if (watch.cancelled()) break;
            ++received;
            if (!session.submit(std::move(frame))) continue; // count stale/out-of-order; never renew capture time
            session.process_one();
            if (watch.cancelled()) break;
            const auto state = session.snapshot();
            if (state.error != Error::none) { error = uint32_t(state.error); break; }
            if (failed_output(sink)) { error = 10; break; }
        }
        if (finished && !error) {
            // Finish is explicit and bounded. Unread/expired final frames are
            // accounted as dropped during close, never mistaken for receipt.
            const auto deadline = GetTickCount64() + PayloadTimeoutMs;
            for (;;) {
                if (watch.cancelled()) break;
                const auto status = sink.status();
                if (failed_output(sink)) { error = 10; break; }
                if ((!status.queued && !status.busy) || GetTickCount64() >= deadline) break;
                Sleep(2);
            }
        }
    } catch (...) { error = uint32_t(Error::exception); }
    if (!finished && !error && !watch.cancelled()) error = 9;
    const auto before_stop = session.snapshot();
    session.stop(); session.drain_stop(); sink.close(); watch.close();
    const bool uncertain = before_stop.cleanup_uncertain || session.snapshot().cleanup_uncertain;
    if (uncertain) error = 8;
    const bool cancelled = watch.cancelled() && !uncertain;
    const auto output = sink.status();
    std::ostringstream json;
    json << "{\"protocol\":3,\"generation\":" << request << ",\"provider\":\"" << provider_name
         << "\",\"width\":" << video::Width << ",\"height\":" << video::Height
         << ",\"received\":" << received << ",\"rejected\":" << before_stop.rejected
         << ",\"provider_dropped\":" << before_stop.dropped << ",\"corrected\":" << output.accepted
         << ",\"output_dropped\":" << output.dropped << ",\"submitted\":" << output.submitted
         << ",\"outcome\":\"" << (cancelled ? "cancelled" : error ? "fault" : "completed")
         << "\",\"error\":" << error << ",\"cleanup_uncertain\":" << (uncertain ? "true" : "false")
         << ",\"camera_started\":false,\"audio_started\":false,\"virtual_device_started\":false}";
    const auto report = json.str();
    return write_small(handles.report, report.data(), DWORD(report.size())) ? 0 : 4;
}
}
