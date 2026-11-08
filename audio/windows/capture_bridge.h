#pragma once
#include <windows.h>
// Optional native input component. No UI supplied pointers/DLL paths. Every
// call and destruction is on the creating thread. Create/Inspect never capture.
struct TxCaptureFormat { UINT32 version, rate, channels, buffer_frames; };
struct TxCapturePacket {
    UINT64 generation, first_frame, qpc_100ns;
    UINT32 frames, flags;
};
struct TxCaptureStatus {
    UINT32 version, phase;
    UINT64 delivered_frames, discarded_frames, packets;
    HRESULT operation_error, release_error, stop_error, reset_error;
    UINT32 reset, startup_discontinuities, reserved[2];
};
extern "C" {
UINT32 WINAPI TxCaptureAbi() noexcept;
void* WINAPI TxCaptureCreate() noexcept;
HRESULT WINAPI TxCaptureInspect(void*, const wchar_t* microphone, UINT32 length,
    const wchar_t* output_return, UINT32 return_length) noexcept;
HRESULT WINAPI TxCapturePrepare(void*, const wchar_t* microphone, UINT32 length,
    const wchar_t* output_return, UINT32 return_length, UINT64 generation, UINT32 explicit_intent,
    TxCaptureFormat*, UINT32 size) noexcept;
HRESULT WINAPI TxCaptureStart(void*, UINT64 generation) noexcept;
// Requires a bounded caller buffer of exactly 4800 float samples. S_FALSE is
// empty. With valid buffer/metadata sizes, failures leave both cleared. Invalid
// argument sizes are rejected without touching memory of unknown capacity.
HRESULT WINAPI TxCaptureRead(void*, UINT64 generation, float*, UINT32 capacity, TxCapturePacket*, UINT32 size) noexcept;
HRESULT WINAPI TxCaptureStop(void*) noexcept;
HRESULT WINAPI TxCaptureStatusRead(void*, TxCaptureStatus*, UINT32 size) noexcept;
HRESULT WINAPI TxCaptureReadyEvent(void*, HANDLE* borrowed_event) noexcept;
HRESULT WINAPI TxCaptureDestroy(void*) noexcept;
}
