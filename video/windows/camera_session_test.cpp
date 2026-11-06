// Fault-injected lifecycle tests plus a real pipe revocation check. No virtual
// camera creation, physical media, GPU, device enumeration or registry writes.
#include "camera_session.h"
#include <array>
#include <future>
#include <iostream>
#include <memory>
#include <stdexcept>

using namespace tingxian::video;
namespace {
int groups = 0;
void expect(bool ok, const char* text) { if (!ok) throw std::runtime_error(text); }
void check(HRESULT hr, const char* text) { expect(SUCCEEDED(hr), text); }
void pass(const char* text) { ++groups; std::cout << "PASS " << text << '\n'; }
enum Call { Prepare, Start, Revoke, Stop, Shutdown };
struct Trace {
    std::array<Call, 128> calls{};
    std::atomic<size_t> size{0};
    void add(Call call) noexcept { const auto i = size.fetch_add(1); if (i < calls.size()) calls[i] = call; }
    bool ends(std::initializer_list<Call> expected) {
        const auto count = size.load(); if (count < expected.size() || count > calls.size()) return false;
        return std::equal(expected.begin(), expected.end(), calls.begin() + count - expected.size());
    }
};
struct Lease : OutputLease {
    Trace& trace;
    FramePublisher* publisher = nullptr;
    std::atomic<bool> revoked{false};
    explicit Lease(Trace& trace) : trace(trace) {}
    void revoke() noexcept override { revoked = true; trace.add(Revoke); if (publisher) publisher->close(); }
};
struct Backend : CameraBackend {
    Trace& trace;
    HRESULT prepare_result = S_OK, start_result = S_OK, stop_result = S_OK, shutdown_result = S_OK;
    Call block = Shutdown;
    HANDLE entered = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    HANDLE release = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    explicit Backend(Trace& trace) : trace(trace) {
        if (!entered || !release) {
            if (entered) CloseHandle(entered); if (release) CloseHandle(release);
            throw std::runtime_error("test events");
        }
    }
    ~Backend() { CloseHandle(entered); CloseHandle(release); }
    HRESULT barrier(Call phase) noexcept {
        if (block != phase) return S_OK;
        SetEvent(entered);
        return WaitForSingleObject(release, 5000) == WAIT_OBJECT_0 ? S_OK : HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    }
    HRESULT prepare(const VideoPipeEndpoint&) noexcept override {
        trace.add(Prepare); const auto hr = barrier(Prepare); return FAILED(hr) ? hr : prepare_result;
    }
    HRESULT start() noexcept override { trace.add(Start); const auto hr = barrier(Start); return FAILED(hr) ? hr : start_result; }
    HRESULT stop() noexcept override { trace.add(Stop); return stop_result; }
    HRESULT shutdown() noexcept override { trace.add(Shutdown); return shutdown_result; }
};
VideoPipeEndpoint endpoint() {
    VideoPipeEndpoint ep;
    wcscpy_s(ep.name, L"\\\\.\\pipe\\TingXian.Video.0123456789abcdef0123456789abcdef");
    ep.producer_pid = GetCurrentProcessId(); ep.capability[0] = 1;
    expect(ProcessIdToSessionId(ep.producer_pid, &ep.producer_session) != FALSE, "process session");
    return ep;
}
struct JoiningThread {
    FramePublisher& publisher;
    std::thread thread;
    JoiningThread(FramePublisher& p, HRESULT& result) : publisher(p), thread([&p, &result] { result = p.accept(); }) {}
    ~JoiningThread() { publisher.close(); if (thread.joinable()) thread.join(); }
};
void run() {
    auto ep = endpoint();
    {
        struct Reservation { HANDLE value = nullptr; ~Reservation() { if (value) CloseHandle(value); } } a, b;
        check(reserve_video_output(&a.value), "first output reservation");
        expect(reserve_video_output(&b.value) == HRESULT_FROM_WIN32(ERROR_BUSY) && !b.value, "second device owner accepted");
        CloseHandle(a.value); a.value = nullptr;
        check(reserve_video_output(&b.value), "reservation after release");
        pass("kernel reservation rejects a second owner and releases without persistent state");
    }
    {
        Trace t; Lease lease(t); Backend backend(t); CameraSession session(backend, lease);
        expect(t.size == 0 && session.status().state == CameraSessionState::idle, "construction started output");
        for (int i = 0; i < 3; ++i) {
            check(session.start(ep), "explicit start");
            expect(session.status().state == CameraSessionState::available && t.ends({Prepare, Start}), "start order");
            const auto before = t.size.load();
            expect(session.start(ep) == HRESULT_FROM_WIN32(ERROR_BUSY) && t.size == before, "duplicate start changed session");
            check(session.stop(), "explicit stop");
            expect(t.ends({Revoke, Stop, Shutdown}) && session.status().state == CameraSessionState::idle, "stop order/state");
        }
        const auto before = t.size.load(); check(session.stop(), "idempotent stop");
        expect(t.size == before + 1 && t.ends({Revoke}), "stop repeated OS work");
        pass("inert construction, three explicit cycles, duplicate start and repeated stop");
    }
    {
        Trace t; Lease lease(t); Backend b(t); CameraSession s(b, lease);
        b.prepare_result = E_ACCESSDENIED;
        expect(s.start(ep) == E_ACCESSDENIED && t.ends({Prepare, Revoke, Stop, Shutdown}), "partial prepare cleanup");
        expect(s.status().operation_error == E_ACCESSDENIED && s.status().cleanup_error == S_OK, "primary error lost");
        pass("partial initialization failure prevents Start and releases acquired resources");
    }
    {
        Trace t; Lease lease(t); Backend b(t); CameraSession s(b, lease);
        b.start_result = E_FAIL;
        expect(s.start(ep) == E_FAIL && t.ends({Start, Revoke, Stop, Shutdown}), "failed Start not cleaned");
        b.start_result = S_OK;
        check(s.start(ep), "explicit recovery");
        expect(s.status().operation_error == S_OK, "recovered state retained old error");
        pass("failed registration cleans up and requires an explicit restart");
    }
    {
        Trace t; Lease lease(t); Backend b(t); CameraSession s(b, lease);
        check(s.start(ep), "start for stop failure"); b.stop_result = E_ACCESSDENIED;
        expect(s.stop() == E_ACCESSDENIED && t.ends({Revoke, Stop, Shutdown}), "Stop failure skipped Shutdown");
        expect(s.status().state == CameraSessionState::fault && s.status().cleanup_error == E_ACCESSDENIED, "false closed state");
        pass("Stop failure still calls Shutdown and remains visible");
    }
    {
        Trace t; Lease lease(t); Backend b(t); CameraSession s(b, lease);
        check(s.start(ep), "start for cleanup failure"); b.shutdown_result = E_FAIL;
        expect(s.stop() == E_FAIL, "shutdown failure hidden");
        const auto before = t.size.load();
        expect(s.start(ep) == HRESULT_FROM_WIN32(ERROR_BUSY) && t.size == before, "failed cleanup handle overwritten");
        b.shutdown_result = S_OK; check(s.stop(), "retry cleanup");
        check(s.start(ep), "restart after cleanup");
        pass("failed Shutdown retains ownership, blocks replacement, permits cleanup retry");
    }
    {
        Trace t; Lease lease(t); Backend b(t); CameraSession s(b, lease);
        b.start_result = E_ACCESSDENIED; b.shutdown_result = E_FAIL;
        expect(s.start(ep) == E_ACCESSDENIED, "cleanup replaced primary error");
        expect(s.status().operation_error == E_ACCESSDENIED && s.status().cleanup_error == E_FAIL, "dual failure not retained");
        b.shutdown_result = S_OK;
        pass("operation and cleanup errors remain distinguishable");
    }
    {
        Trace t; Lease lease(t); Backend b(t); CameraSession s(b, lease);
        auto invalid = ep; wcscpy_s(invalid.name, L"\\\\remote\\pipe\\camera");
        expect(s.start(invalid) == E_INVALIDARG && t.ends({Revoke}) && t.size == 1, "invalid endpoint reached OS");
        pass("invalid or remote endpoint rejected before any device operation");
    }
    {
        Trace t; Lease lease(t); Backend b(t);
        { CameraSession s(b, lease); check(s.start(ep), "start for destructor"); }
        expect(t.ends({Revoke, Stop, Shutdown}), "destruction did not revoke output before OS cleanup");
        pass("owner destruction revokes frames and closes its device session");
    }
    for (Call phase : {Prepare, Start}) {
        Trace t; Lease lease(t); Backend b(t); CameraSession s(b, lease); b.block = phase;
        auto pending = std::async(std::launch::async, [&] { return s.start(ep); });
        const auto waiting = WaitForSingleObject(b.entered, 2000);
        if (waiting != WAIT_OBJECT_0) { SetEvent(b.release); pending.wait(); throw std::runtime_error("start barrier"); }
        s.request_stop();
        const bool revoked_before_release = lease.revoked.load();
        SetEvent(b.release);
        expect(pending.get() == HRESULT_FROM_WIN32(ERROR_CANCELLED), "late successful start escaped cancellation");
        expect(revoked_before_release && t.ends({Revoke, Stop, Shutdown}), "cancel did not revoke before OS completed");
        if (phase == Prepare) expect(std::find(t.calls.begin(), t.calls.begin() + t.size, Start) == t.calls.begin() + t.size, "cancelled prepare still called Start");
        pass(phase == Prepare ? "cancel while prepare is blocked prevents device Start" : "cancel while Start is blocked cleans its late result");
    }
    {
        std::unique_ptr<CameraBackend> backend(make_windows_camera_backend()); expect(bool(backend), "backend allocation");
        expect(backend->prepare(ep) == CO_E_NOTINITIALIZED, "native backend used uninitialized COM");
        check(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED), "STA test setup");
        const HRESULT sta = backend->prepare(ep); CoUninitialize();
        expect(sta == RPC_E_WRONG_THREAD, "native backend allowed UI STA");
        pass("production backend rejects absent COM and UI STA before device access");
    }
    {
        Trace t; Lease lease(t); Backend b(t);
        FramePublisher publisher; lease.publisher = &publisher;
        VideoPipeEndpoint actual; check(publisher.open(&actual), "real pipe open");
        HRESULT accepted = E_UNEXPECTED;
        JoiningThread accept(publisher, accepted);
        auto mailbox = std::make_shared<FrameMailbox>(); PipeReceiver receiver(mailbox);
        check(receiver.connect(actual), "real receiver connect"); accept.thread.join(); check(accepted, "real handshake");
        CameraSession s(b, lease); check(s.start(actual), "session for active producer");
        std::vector<BYTE> frame(FrameBytes, 160), output(FrameBytes);
        check(publisher.begin(1), "real begin");
        check(publisher.submit(1, 1, GetTickCount64(), frame.data(), FrameBytes), "real frame");
        auto deadline = GetTickCount64() + 2000;
        while (mailbox->metrics().accepted == 0 && GetTickCount64() < deadline) Sleep(1);
        expect(mailbox->metrics().accepted == 1, "frame did not arrive before stop");
        b.stop_result = E_ACCESSDENIED; expect(s.stop() == E_ACCESSDENIED, "inject OS stop failure");
        deadline = GetTickCount64() + 2000;
        while (receiver.status().state != TransportState::fault && GetTickCount64() < deadline) Sleep(1);
        expect(receiver.status().state == TransportState::fault && !mailbox->metrics().enabled, "OS failure left transport live");
        check(mailbox->write(output.data(), Width, output.data(), FrameBytes, [] { return S_OK; }), "neutral output");
        expect(std::all_of(output.begin(), output.begin() + Width * Height, [](BYTE b) { return b == 32; })
            && std::all_of(output.begin() + Width * Height, output.end(), [](BYTE b) { return b == 128; }), "retained frame after revocation");
        pass("real pipe closes and yields neutral pixels even when OS device Stop fails");
    }
}
}
int main() {
    try {
        run();
        std::cout << "{\"checks\":" << groups << ",\"result\":\"pass\",\"os_lifecycle\":\"injected\","
            "\"real_pipe_revocation\":true,\"device_registered\":false,\"physical_camera_opened\":false}\n";
        return 0;
    } catch (const std::exception& error) { std::cerr << "FAIL " << error.what() << '\n'; return 1; }
}
