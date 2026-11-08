#include "device_bridge_internal.h"
#include <cstring>
using namespace tingxian::video;
namespace {
struct Lease final : OutputLease {
    TxDeviceRevoke callback;
    void* context;
    std::atomic<bool> revoked{false};
    Lease(TxDeviceRevoke cb, void* ctx) : callback(cb), context(ctx) {}
    void revoke() noexcept override {
        if (!revoked.exchange(true)) callback(context);
    }
};
struct Instance {
    DWORD thread = GetCurrentThreadId();
    std::unique_ptr<CameraBackend> backend;
    Lease lease;
    CameraSession session;
    const UINT64 ticket;
    std::atomic<bool> used{false};
    Instance(std::unique_ptr<CameraBackend> b, TxDeviceRevoke cb, void* ctx)
        : backend(std::move(b)), lease(cb, ctx), session(*backend, lease),
          ticket(session.operation_ticket()) {}
};
HRESULT owner(void* raw) noexcept {
    return !raw ? E_INVALIDARG : static_cast<Instance*>(raw)->thread == GetCurrentThreadId()
        ? S_OK : RPC_E_WRONG_THREAD;
}
}
void* tingxian::video::make_device_bridge(std::unique_ptr<CameraBackend> b,
    TxDeviceRevoke callback, void* context) noexcept {
    if (!b || !callback || !context) return nullptr;
    try { return new Instance(std::move(b), callback, context); } catch (...) { return nullptr; }
}
extern "C" {
UINT32 WINAPI TxDeviceAbi() noexcept { return 1; }
void* WINAPI TxDeviceCreate(TxDeviceRevoke cb, void* ctx) noexcept {
    if (!cb || !ctx) return nullptr;
    return make_device_bridge(std::unique_ptr<CameraBackend>(make_windows_camera_backend()), cb, ctx);
}
HRESULT WINAPI TxDeviceStart(void* raw, const BYTE* bytes, UINT32 length, UINT32 intent) noexcept {
    const auto hr = owner(raw); if (FAILED(hr)) return hr;
    auto& i = *static_cast<Instance*>(raw);
    if (i.used.exchange(true)) return HRESULT_FROM_WIN32(ERROR_BUSY);
    if (intent != 1 || !bytes || length != sizeof(VideoPipeEndpoint)) {
        i.session.request_stop(); return intent != 1 ? E_ACCESSDENIED : E_INVALIDARG;
    }
    VideoPipeEndpoint ep{}; std::memcpy(&ep, bytes, sizeof(ep));
    HRESULT result = E_INVALIDARG;
    if (ep.version == 2 && valid_video_endpoint(ep)) result = i.session.start_if_current(ep, i.ticket);
    else i.session.request_stop();
    SecureZeroMemory(&ep, sizeof(ep)); return result;
}
void WINAPI TxDeviceCancel(void* raw) noexcept {
    if (raw) static_cast<Instance*>(raw)->session.request_stop();
}
HRESULT WINAPI TxDeviceStatusGet(void* raw, TxDeviceStatus* status, UINT32 size) noexcept {
    if (!raw || !status || size != sizeof(*status)) return E_INVALIDARG;
    auto& i = *static_cast<Instance*>(raw); const auto s = i.session.status();
    *status = {1, static_cast<UINT32>(s.state), s.operation_error, s.cleanup_error,
        i.lease.revoked.load() ? 1u : 0u, i.used.load() ? 1u : 0u}; return S_OK;
}
HRESULT WINAPI TxDeviceStop(void* raw) noexcept {
    const auto hr = owner(raw); if (FAILED(hr)) return hr;
    return static_cast<Instance*>(raw)->session.stop();
}
HRESULT WINAPI TxDeviceDestroy(void* raw) noexcept {
    const auto hr = owner(raw); if (FAILED(hr)) return hr;
    auto* i = static_cast<Instance*>(raw);
    const auto stopped = i->session.stop();
    if (FAILED(stopped)) return stopped;
    delete i; return S_OK;
}
}
