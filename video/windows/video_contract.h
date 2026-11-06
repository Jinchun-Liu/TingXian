#pragma once
#include <windows.h>
#include <unknwn.h>
#include <cstdint>

namespace tingxian::video {
inline constexpr UINT32 Width = 1280;
inline constexpr UINT32 Height = 720;
inline constexpr UINT32 FrameBytes = Width * Height * 3 / 2;
inline constexpr UINT64 MaxFrameAgeMs = 250;
inline constexpr LONGLONG FrameDuration = 10000000 / 30;
// A new CLSID; never register the Microsoft sample's identity.
inline constexpr GUID SourceClass = {0x44d20da7, 0x55ec, 0x4b4a, {0xa7,0x67,0x99,0xe9,0x7d,0x6e,0x5a,0x90}};

struct FrameMetrics {
    UINT64 accepted = 0;
    UINT64 rejected = 0;
    UINT64 dropped = 0;
    UINT64 unique_samples = 0;
    UINT64 repeated_samples = 0;
    UINT64 neutral_samples = 0;
    UINT64 generation = 0;
    BOOL enabled = FALSE;
};

// In-process adapter boundary. Not a registered marshalled COM interface or IPC.
// A future authenticated transport in the media-source process will call this.
// These calls do not open a physical camera or process eyes.
MIDL_INTERFACE("f4ac483c-1bee-4e1f-961a-4f8a03fd1580") IFrameReceiver : public IUnknown {
    virtual HRESULT STDMETHODCALLTYPE BeginVideo(UINT64* generation) = 0;
    virtual HRESULT STDMETHODCALLTYPE SubmitNv12(UINT64 generation, UINT64 frame_id,
        UINT64 captured_at_ms, const BYTE* bytes, UINT32 length) = 0;
    virtual HRESULT STDMETHODCALLTYPE StopVideo() = 0;
    virtual HRESULT STDMETHODCALLTYPE GetFrameMetrics(FrameMetrics* metrics) = 0;
};
}
