#include "input_bridge.h"
#include "input_camera.h"
#include <algorithm>
#include <new>
using namespace tingxian::camera_input;
static_assert(sizeof(TxCameraFrame) == 32);
static_assert(sizeof(TxCameraStatus) == 104);
namespace {
struct Owner {
    const DWORD thread = GetCurrentThreadId();
    CameraInput input;
    UINT64 generation = 0;
};
Owner* owner(void* raw) noexcept { return static_cast<Owner*>(raw); }
HRESULT valid(void* raw) noexcept {
    return !raw ? E_POINTER : owner(raw)->thread != GetCurrentThreadId() ? RPC_E_WRONG_THREAD : S_OK;
}
}
extern "C" {
UINT32 WINAPI TxCameraAbi() noexcept { return 1; }
void* WINAPI TxCameraCreate() noexcept { try { return new Owner; } catch (...) { return nullptr; } }
HRESULT WINAPI TxCameraPrepare(void* raw, const wchar_t* link, UINT32 length, UINT64 generation, UINT32 intent) noexcept {
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (!intent) return E_ACCESSDENIED;
    if (intent != 1 || !link || !length || length > 32767 || !generation || std::find(link, link + length, L'\0') != link + length) return E_INVALIDARG;
    try {
        const auto result = owner(raw)->input.prepare(std::wstring(link, length), generation, true);
        if (SUCCEEDED(result)) owner(raw)->generation = generation;
        return result;
    } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; } catch (...) { return E_FAIL; }
}
HRESULT WINAPI TxCameraStart(void* raw, UINT64 generation) noexcept {
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (!generation || generation != owner(raw)->generation) return E_INVALIDARG;
    return owner(raw)->input.start();
}
HRESULT WINAPI TxCameraRead(void* raw, UINT64 generation, BYTE* bgr, UINT32 capacity, TxCameraFrame* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    if (!bgr || capacity != BgrBytes) return E_INVALIDARG;
    SecureZeroMemory(bgr, capacity);
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (!generation || generation != owner(raw)->generation) return E_INVALIDARG;
    Frame frame; const auto result = owner(raw)->input.read(frame);
    if (result == S_OK) {
        if (frame.generation != generation || frame.bgr.size() != capacity) return E_UNEXPECTED;
        std::copy(frame.bgr.begin(), frame.bgr.end(), bgr);
        *out = {frame.generation, frame.sequence, frame.capture_qpc, capacity, frame.discontinuity};
    }
    return result;
}
HRESULT WINAPI TxCameraRevoke(void* raw) noexcept { const auto hr = valid(raw); return FAILED(hr) ? hr : owner(raw)->input.revoke(); }
HRESULT WINAPI TxCameraStop(void* raw) noexcept { const auto hr = valid(raw); return FAILED(hr) ? hr : owner(raw)->input.stop(); }
HRESULT WINAPI TxCameraStatusRead(void* raw, TxCameraStatus* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    const auto s = owner(raw)->input.status(); const auto f = s.frames;
    *out = {1, UINT32(s.phase), f.accepted, f.delivered, f.stale, f.superseded, f.cleared, f.late, f.discontinuities,
        f.queued, f.closed ? 1u : 0u, s.cleanup_uncertain ? 1u : 0u, 0,
        s.operation, f.error, s.flush, s.shutdown, s.release, 0};
    return S_OK;
}
HRESULT WINAPI TxCameraReadyEvent(void* raw, HANDLE* out) noexcept {
    if (!out) return E_POINTER; *out = nullptr;
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    *out = owner(raw)->input.ready_event(); return S_OK;
}
HRESULT WINAPI TxCameraDestroy(void* raw) noexcept {
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    const auto result = owner(raw)->input.stop(); if (FAILED(result)) return result;
    delete owner(raw); return S_OK;
}
}
