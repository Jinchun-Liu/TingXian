#include "output_bridge.h"
#include "eye_output.h"
#include <new>
#include <cstring>
using tingxian::video::EyePipeOutput;
using tingxian::video::VideoPipeEndpoint;
static_assert(sizeof(TxOutputStatus) == 64);
static_assert(sizeof(VideoPipeEndpoint) == 236);
extern "C" {
UINT32 WINAPI TxOutputAbi() noexcept { return 1; }
void* WINAPI TxOutputCreate() noexcept { try { return new EyePipeOutput; } catch (...) { return nullptr; } }
HRESULT WINAPI TxOutputOpen(void* raw, UINT64 generation, BYTE* bytes, UINT32 size) noexcept {
    if (!raw || !bytes || size != sizeof(VideoPipeEndpoint)) return E_INVALIDARG;
    SecureZeroMemory(bytes, size);
    auto& output = *static_cast<EyePipeOutput*>(raw); VideoPipeEndpoint endpoint{};
    const auto hr = output.open_if_current(generation, output.operation_ticket(), &endpoint);
    if (SUCCEEDED(hr)) std::memcpy(bytes, &endpoint, size);
    SecureZeroMemory(&endpoint, sizeof(endpoint)); return hr;
}
HRESULT WINAPI TxOutputSubmit(void* raw, UINT64 generation, UINT64 id, UINT64 captured, const BYTE* data, UINT32 size) noexcept {
    if (!raw) return E_INVALIDARG;
    return static_cast<EyePipeOutput*>(raw)->corrected_nv12(generation, id, captured, data, size) ? S_OK : E_INVALIDARG;
}
void WINAPI TxOutputCancel(void* raw) noexcept { if (raw) static_cast<EyePipeOutput*>(raw)->request_stop(); }
void WINAPI TxOutputClose(void* raw) noexcept { if (raw) static_cast<EyePipeOutput*>(raw)->close(); }
HRESULT WINAPI TxOutputGetStatus(void* raw, TxOutputStatus* status, UINT32 size) noexcept {
    if (!raw || !status || size != sizeof(*status)) return E_INVALIDARG;
    const auto s = static_cast<EyePipeOutput*>(raw)->status();
    *status = {1, UINT32(s.state), s.generation, s.accepted, s.rejected, s.dropped, s.submitted,
        UINT32(s.queued), s.busy ? 1u : 0u, s.error, 0}; return S_OK;
}
void WINAPI TxOutputDestroy(void* raw) noexcept { delete static_cast<EyePipeOutput*>(raw); }
}
