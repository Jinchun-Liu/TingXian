#include "capture_bridge.h"
#include "capture_input.h"
#include <algorithm>
#include <new>
using namespace tingxian::capture;
static_assert(sizeof(TxCaptureFormat) == 16);
static_assert(sizeof(TxCapturePacket) == 32);
static_assert(sizeof(TxCaptureStatus) == 64);
namespace {
struct Owner {
    const DWORD thread = GetCurrentThreadId();
    std::unique_ptr<Backend> backend = make_windows_backend();
    Session session{*backend};
};
Owner* owner(void* raw) noexcept { return static_cast<Owner*>(raw); }
HRESULT valid(void* raw) noexcept {
    return !raw ? E_POINTER : owner(raw)->thread != GetCurrentThreadId() ? RPC_E_WRONG_THREAD : S_OK;
}
bool id(const wchar_t* p, UINT32 size) noexcept {
    return p && size && size <= 1024 && std::find(p, p + size, L'\0') == p + size;
}
}
extern "C" {
UINT32 WINAPI TxCaptureAbi() noexcept { return 1; }
void* WINAPI TxCaptureCreate() noexcept {
    try { return new Owner; } catch (...) { return nullptr; }
}
HRESULT WINAPI TxCaptureInspect(void* raw, const wchar_t* mic, UINT32 size, const wchar_t* ret, UINT32 rsize) noexcept {
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (owner(raw)->session.status().phase != Phase::idle) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    if (!id(mic, size) || !id(ret, rsize)) return E_INVALIDARG;
    try { return owner(raw)->backend->inspect({std::wstring(mic, size), std::wstring(ret, rsize)}); }
    catch (const std::bad_alloc&) { return E_OUTOFMEMORY; } catch (...) { return E_FAIL; }
}
HRESULT WINAPI TxCapturePrepare(void* raw, const wchar_t* mic, UINT32 length, const wchar_t* ret, UINT32 rlength,
    UINT64 generation, UINT32 intent, TxCaptureFormat* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (!id(mic, length) || !id(ret, rlength) || intent > 1) return E_INVALIDARG;
    try {
        auto& s = owner(raw)->session;
        const auto result = s.prepare({std::wstring(mic, length), std::wstring(ret, rlength)}, generation, intent == 1);
        if (result == S_OK) { const auto f = s.format(); *out = {1, f.rate, f.channels, f.buffer_frames}; }
        return result;
    } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; } catch (...) { return E_FAIL; }
}
HRESULT WINAPI TxCaptureStart(void* raw, UINT64 generation) noexcept {
    const auto hr = valid(raw); return FAILED(hr) ? hr : owner(raw)->session.start(generation);
}
HRESULT WINAPI TxCaptureRead(void* raw, UINT64 generation, float* data, UINT32 capacity, TxCapturePacket* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    if (!data || capacity != max_frames) return E_INVALIDARG;
    SecureZeroMemory(data, capacity * sizeof(float));
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    Packet p;
    const auto result = owner(raw)->session.read(generation, p);
    if (result == S_OK) {
        std::copy_n(p.samples.data(), p.frames, data);
        *out = {p.generation, p.first_frame, p.qpc_100ns, p.frames, p.flags};
    }
    return result;
}
HRESULT WINAPI TxCaptureStop(void* raw) noexcept {
    const auto hr = valid(raw); return FAILED(hr) ? hr : owner(raw)->session.stop();
}
HRESULT WINAPI TxCaptureStatusRead(void* raw, TxCaptureStatus* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    const auto s = owner(raw)->session.status();
    *out = {1, UINT32(s.phase), s.delivered_frames, s.discarded_frames, s.packets, s.operation_error,
        s.release_error, s.cleanup.stop_error, s.cleanup.reset_error, s.cleanup.reset ? 1u : 0u,
        s.startup_discontinuities, {0, 0}};
    return S_OK;
}
HRESULT WINAPI TxCaptureReadyEvent(void* raw, HANDLE* out) noexcept {
    if (!out) return E_POINTER;
    *out = nullptr;
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    *out = owner(raw)->backend->ready_event(); return S_OK;
}
HRESULT WINAPI TxCaptureDestroy(void* raw) noexcept {
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    delete owner(raw); return S_OK;
}
}
