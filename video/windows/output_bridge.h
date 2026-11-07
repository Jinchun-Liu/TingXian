#pragma once
#include <windows.h>
// Native-only, fixed ABI. No COM registration, device, SDK or implicit start.
// An owning native lane serializes Open/Submit/Close. Cancel/Status are thread safe.
// Destroy must follow Close and the retirement of all concurrent calls.
struct TxOutputStatus {
    UINT32 version, state;
    UINT64 generation, accepted, rejected, dropped, submitted;
    UINT32 queued, busy;
    HRESULT error;
    UINT32 reserved;
};
extern "C" {
UINT32 WINAPI TxOutputAbi() noexcept;
void* WINAPI TxOutputCreate() noexcept;
HRESULT WINAPI TxOutputOpen(void*, UINT64, BYTE*, UINT32) noexcept;
HRESULT WINAPI TxOutputSubmit(void*, UINT64, UINT64, UINT64, const BYTE*, UINT32) noexcept;
void WINAPI TxOutputCancel(void*) noexcept;
void WINAPI TxOutputClose(void*) noexcept;
HRESULT WINAPI TxOutputGetStatus(void*, TxOutputStatus*, UINT32) noexcept;
void WINAPI TxOutputDestroy(void*) noexcept;
}
