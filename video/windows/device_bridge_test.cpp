// Injected OS boundary only. No actual virtual device or physical media.
#include "device_bridge_internal.h"
#include <future>
#include <iostream>
#include <stdexcept>
using namespace tingxian::video;
namespace {
void expect(bool yes, const char* why) { if (!yes) throw std::runtime_error(why); }
struct State {
    std::atomic<int> prepare{0}, start{0}, stop{0}, shutdown{0}, revoked{0}, destroyed{0};
    HRESULT prepare_result = S_OK, stop_result = S_OK, shutdown_result = S_OK;
    int block = 0;
    HANDLE entered = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    HANDLE release = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    ~State() { if (entered) CloseHandle(entered); if (release) CloseHandle(release); }
};
struct Backend : CameraBackend {
    State& s;
    explicit Backend(State& value) : s(value) {}
    ~Backend() { ++s.destroyed; }
    HRESULT gate(int n) noexcept {
        if (s.block != n) return S_OK;
        SetEvent(s.entered);
        return WaitForSingleObject(s.release, 3000) == WAIT_OBJECT_0 ? S_OK : E_ABORT;
    }
    HRESULT prepare(const VideoPipeEndpoint&) noexcept override {
        ++s.prepare; const auto hr = gate(1); return FAILED(hr) ? hr : s.prepare_result;
    }
    HRESULT start() noexcept override { ++s.start; return gate(2); }
    HRESULT stop() noexcept override { ++s.stop; return s.stop_result; }
    HRESULT shutdown() noexcept override { ++s.shutdown; return s.shutdown_result; }
};
void WINAPI revoke(void* p) { ++static_cast<State*>(p)->revoked; }
void* create(State& s) { return make_device_bridge(std::make_unique<Backend>(s), revoke, &s); }
VideoPipeEndpoint endpoint() {
    VideoPipeEndpoint ep{}; ep.version = 2;
    wcscpy_s(ep.name, L"\\\\.\\pipe\\TingXian.Video.0123456789abcdef0123456789abcdef");
    ep.producer_pid = GetCurrentProcessId(); ep.capability[0] = 1;
    expect(ProcessIdToSessionId(ep.producer_pid, &ep.producer_session) != 0, "session");
    return ep;
}
HRESULT start(void* raw, const VideoPipeEndpoint& ep, UINT32 intent = 1) {
    return TxDeviceStart(raw, reinterpret_cast<const BYTE*>(&ep), sizeof(ep), intent);
}
TxDeviceStatus status(void* raw) {
    TxDeviceStatus s{}; expect(TxDeviceStatusGet(raw, &s, sizeof(s)) == S_OK, "status"); return s;
}
void run() {
    auto ep = endpoint();
    expect(TxDeviceAbi() == 1 && !TxDeviceCreate(nullptr, nullptr), "ABI/callback");
    {
        State s; auto* raw = create(s); expect(raw && s.prepare == 0, "inert create");
        expect(status(raw).state == 0 && status(raw).used == 0, "initial state");
        const auto wrong = std::async(std::launch::async, [&] {
            return start(raw, ep) == RPC_E_WRONG_THREAD && TxDeviceStop(raw) == RPC_E_WRONG_THREAD
                && TxDeviceDestroy(raw) == RPC_E_WRONG_THREAD;
        }).get();
        expect(wrong && s.prepare == 0 && status(raw).used == 0, "owner thread gate");
        expect(start(raw, ep) == S_OK && status(raw).state == 2, "v2 start");
        expect(start(raw, ep) == HRESULT_FROM_WIN32(ERROR_BUSY) && s.start == 1, "single start");
        expect(TxDeviceStop(raw) == S_OK && s.revoked == 1 && s.shutdown == 1, "stop");
        expect(TxDeviceDestroy(raw) == S_OK && s.destroyed == 1 && s.revoked == 1, "destroy");
    }
    std::cout << "PASS inert create, v2 start, owner thread and single-use lifecycle\n";
    for (UINT32 version : {1u, 3u, 99u}) {
        State s; auto* raw = create(s); auto bad = ep; bad.version = version;
        expect(start(raw, bad) == E_INVALIDARG && s.prepare == 0 && s.revoked == 1, "version denial");
        expect(TxDeviceDestroy(raw) == S_OK, "version destroy");
    }
    for (UINT32 intent : {0u, 2u}) {
        State s; auto* raw = create(s);
        expect(start(raw, ep, intent) == E_ACCESSDENIED && s.prepare == 0 && s.revoked == 1, "intent denial");
        expect(TxDeviceDestroy(raw) == S_OK, "intent destroy");
    }
    {
        State s; auto* raw = create(s);
        expect(TxDeviceStart(raw, nullptr, 236, 1) == E_INVALIDARG && s.prepare == 0 && s.revoked == 1, "null endpoint");
        expect(TxDeviceDestroy(raw) == S_OK, "null destroy");
    }
    std::cout << "PASS unsupported endpoints and unauthorized start revoke before OS calls\n";
    {
        State s; auto* raw = create(s); TxDeviceCancel(raw);
        expect(start(raw, ep) == HRESULT_FROM_WIN32(ERROR_CANCELLED) && s.prepare == 0, "queued cancel");
        expect(TxDeviceDestroy(raw) == S_OK && s.revoked == 1, "cancel destroy");
    }
    for (int phase : {1, 2}) {
        State s; s.block = phase;
        std::promise<void*> ready; auto pointer = ready.get_future();
        auto worker = std::async(std::launch::async, [&] {
            auto* raw = create(s); ready.set_value(raw);
            const auto hr = start(raw, ep); const auto st = status(raw);
            const auto cleanup = TxDeviceDestroy(raw);
            return hr == HRESULT_FROM_WIN32(ERROR_CANCELLED) && st.state == 4
                && st.operation == hr && st.cleanup == S_OK && cleanup == S_OK;
        });
        auto* raw = pointer.get();
        expect(WaitForSingleObject(s.entered, 2000) == WAIT_OBJECT_0, "blocked phase");
        TxDeviceCancel(raw);
        expect(s.revoked == 1 && s.shutdown == 0, "revocation must precede slow OS return");
        SetEvent(s.release);
        expect(worker.get() && s.shutdown == 1 && s.start == (phase == 1 ? 0 : 1), "late start cleanup");
    }
    std::cout << "PASS queued, prepare and start cancellation revoke before OS return\n";
    {
        State s; auto* raw = create(s); expect(start(raw, ep) == S_OK, "cleanup start");
        s.stop_result = E_ACCESSDENIED; s.shutdown_result = E_FAIL;
        expect(TxDeviceDestroy(raw) == E_ACCESSDENIED && s.destroyed == 0 && s.shutdown == 1, "failed destroy retained");
        expect(status(raw).cleanup == E_ACCESSDENIED && status(raw).state == 4 && s.revoked == 1, "cleanup failure visible");
        expect(start(raw, ep) == HRESULT_FROM_WIN32(ERROR_BUSY), "failed destroy cannot restart");
        s.stop_result = S_OK; s.shutdown_result = S_OK;
        expect(TxDeviceDestroy(raw) == S_OK && s.destroyed == 1 && s.shutdown == 2 && s.revoked == 1, "explicit cleanup retry");
    }
    {
        State s; s.prepare_result = E_ACCESSDENIED; s.shutdown_result = E_FAIL;
        auto* raw = create(s); expect(start(raw, ep) == E_ACCESSDENIED, "prepare failure");
        auto st = status(raw);
        expect(st.operation == E_ACCESSDENIED && st.cleanup == E_FAIL && s.start == 0, "distinct primary cleanup errors");
        expect(TxDeviceDestroy(raw) == E_FAIL && s.destroyed == 0, "retain partial prepare");
        s.shutdown_result = S_OK; expect(TxDeviceDestroy(raw) == S_OK, "partial retry");
    }
    std::cout << "PASS cleanup failure retains owner and callback, original and cleanup errors distinct\n";
}
}
int main() {
    try { run(); return 0; } catch (const std::exception& e) { std::cerr << e.what() << '\n'; return 1; }
}
