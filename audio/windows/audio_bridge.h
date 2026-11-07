#pragma once
#include <windows.h>
// Native host only; no webview-supplied DLL, endpoint, pointer or callback.
// One owner thread serializes every call including GetStatus and Destroy.
// All structs use exact byte size/version/reserved checks, no C++ ABI types.
struct TxAudioFormat { UINT32 version, rate, channels, buffer_frames; UINT64 clock_frequency; };
struct TxAudioClock { UINT64 position, qpc_100ns; UINT32 padding, reserved; };
struct TxAudioTicket {
    UINT64 generation, serial, first_frame;
    UINT32 frames, reserved;
    TxAudioClock clock;
};
struct TxAudioStatus {
    UINT32 version, phase;
    UINT64 submitted_frames;
    HRESULT operation_error, stop_error, reset_error;
    UINT32 client_reset, external_drain, revoked, reserved[2];
};
// Called synchronously before OS cleanup. Must not unwind/reenter or block.
using TxAudioRevoke = void (WINAPI*)(void* context);
extern "C" {
UINT32 WINAPI TxAudioAbi() noexcept;
void* WINAPI TxAudioCreate(TxAudioRevoke revoke, void* context) noexcept;
// Metadata only; does not prepare a client, prime PCM, set attempted, or revoke.
HRESULT WINAPI TxAudioInspectPair(void*, const wchar_t* render, UINT32 render_length,
    const wchar_t* capture, UINT32 capture_length) noexcept;
HRESULT WINAPI TxAudioPrepare(void*, const wchar_t* render, UINT32 render_length,
    const wchar_t* capture, UINT32 capture_length, UINT32 channels, UINT64 generation,
    TxAudioFormat*, UINT32 size) noexcept;
HRESULT WINAPI TxAudioStart(void*, UINT64 generation, UINT32 explicit_intent) noexcept;
HRESULT WINAPI TxAudioObserve(void*, UINT64 generation, TxAudioClock*, UINT32 size) noexcept;
HRESULT WINAPI TxAudioReserve(void*, UINT64 generation, UINT32 frames, TxAudioTicket*, UINT32 size) noexcept;
HRESULT WINAPI TxAudioCommit(void*, const TxAudioTicket*, UINT32 size, UINT32 source, const float*, UINT32 frames) noexcept;
HRESULT WINAPI TxAudioStop(void*) noexcept;
HRESULT WINAPI TxAudioGetStatus(void*, TxAudioStatus*, UINT32 size) noexcept;
HRESULT WINAPI TxAudioReadyEvent(void*, HANDLE* borrowed_event) noexcept;
HRESULT WINAPI TxAudioDestroy(void*) noexcept;
}
