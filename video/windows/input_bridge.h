#pragma once
#include <windows.h>
// Optional native-only ABI; all instance calls on their creating MTA lane.
// No interface here authorizes display/UI pointers, DLL paths or restored starts.
struct TxCameraFrame { UINT64 generation, sequence, qpc_100ns; UINT32 bytes, discontinuity; };
struct TxCameraStatus {
    UINT32 version, phase;
    UINT64 accepted, delivered, stale, superseded, cleared, late, discontinuities;
    UINT32 queued, closed, cleanup_uncertain, reserved;
    HRESULT operation, frame_error, flush, shutdown, release;
    UINT32 reserved2;
};
extern "C" {
UINT32 WINAPI TxCameraAbi() noexcept;
void* WINAPI TxCameraCreate() noexcept;
HRESULT WINAPI TxCameraPrepare(void*, const wchar_t* selected, UINT32 length, UINT64 generation, UINT32 intent) noexcept;
HRESULT WINAPI TxCameraStart(void*, UINT64 generation) noexcept;
HRESULT WINAPI TxCameraRead(void*, UINT64 generation, BYTE* bgr, UINT32 capacity, TxCameraFrame*, UINT32 size) noexcept;
HRESULT WINAPI TxCameraRevoke(void*) noexcept;
HRESULT WINAPI TxCameraStop(void*) noexcept;
HRESULT WINAPI TxCameraStatusRead(void*, TxCameraStatus*, UINT32 size) noexcept;
HRESULT WINAPI TxCameraReadyEvent(void*, HANDLE*) noexcept;
// Failure leaves the instance allocated: never claim destruction under uncertain
// cleanup. Caller retains its file/library pin and disallows another session.
HRESULT WINAPI TxCameraDestroy(void*) noexcept;
}
