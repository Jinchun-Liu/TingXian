#pragma once
#include <windows.h>

// Native controller only. Callback is thread safe, nonblocking and noexcept;
// revoke BOTH media inputs/outputs, not just this virtual camera. The callback
// and its context must survive until a successful TxDeviceDestroy. Never call
// back into this instance. Cancel/status may race owner operations, never destroy.
using TxDeviceRevoke = void (WINAPI*)(void*);
struct TxDeviceStatus {
    UINT32 version, state;
    HRESULT operation, cleanup;
    UINT32 revoked, used;
};
static_assert(sizeof(TxDeviceStatus) == 24);
extern "C" {
UINT32 WINAPI TxDeviceAbi() noexcept;
void* WINAPI TxDeviceCreate(TxDeviceRevoke revoke, void* context) noexcept;
// One attempt per instance. v2 only; no worker v3 or legacy untimed endpoint.
// MTA ownership is caller-managed. Intent must be exactly 1.
HRESULT WINAPI TxDeviceStart(void*, const BYTE*, UINT32 length, UINT32 intent) noexcept;
void WINAPI TxDeviceCancel(void*) noexcept;
HRESULT WINAPI TxDeviceStatusGet(void*, TxDeviceStatus*, UINT32 size) noexcept;
HRESULT WINAPI TxDeviceStop(void*) noexcept;
// Failed cleanup leaves the pointer VALID: retain module/context and retry stop.
HRESULT WINAPI TxDeviceDestroy(void*) noexcept;
}
