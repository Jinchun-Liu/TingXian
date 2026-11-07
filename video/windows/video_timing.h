#pragma once
#include "video_contract.h"
#include <climits>

namespace tingxian::video {
inline constexpr UINT32 TimedQueueCapacity = 4;
inline constexpr UINT64 TimedHorizon = 1000000; // 100 ms maximum planned lead.
inline constexpr UINT64 TimedMaxAge = MaxFrameAgeMs * 10000;
inline constexpr UINT32 TimedNeutral = 1;
inline constexpr UINT32 TimedRepeated = 2; // Output sample metadata only.
// v2 only: no conversion/reinterpretation of v1 captured_at_ms.
struct TimedFrame {
    UINT64 frame_id = 0, capture_qpc = 0, presentation_qpc = 0;
    UINT32 flags = 0, reserved = 0;
};
static_assert(sizeof(TimedFrame) == 32);
struct TimedMetrics {
    UINT32 version = 2, queued = 0;
    UINT32 enabled = 0, reserved = 0;
    UINT64 generation = 0;
    UINT64 epoch = 0, accepted = 0, rejected = 0, late = 0, superseded = 0;
    UINT64 unique = 0, repeated = 0, neutral = 0, cleared = 0;
};
static_assert(sizeof(TimedMetrics) == 96);
// Attached to actual MF samples for native diagnostics, not guaranteed to survive
// a meeting client's processing. presentation_qpc stays the original plan even
// for a repeated sample whose MF sample time advances to the repeat time.
struct TimedSample {
    UINT32 version = 2, flags = TimedNeutral;
    UINT64 epoch = 0, sequence = 0, frame_id = 0, capture_qpc = 0;
    UINT64 presentation_qpc = 0, delivered_qpc = 0;
};
static_assert(sizeof(TimedSample) == 56);
inline constexpr GUID SampleTimingAttribute = {0x905e4e21,0x467f,0x4f52,{0xb3,0xa1,0x94,0xc2,0xe5,0x72,0x45,0x8c}};
MIDL_INTERFACE("b9d4225a-3f49-4fa0-a564-7e1ed4dcc942") ITimedFrameReceiver : public IUnknown {
    virtual HRESULT STDMETHODCALLTYPE GetTimingVersion(UINT32* version) = 0;
    virtual HRESULT STDMETHODCALLTYPE BeginTimedVideo(UINT64 epoch, UINT64* generation) = 0;
    virtual HRESULT STDMETHODCALLTYPE SubmitTimedNv12(UINT64 generation, UINT64 sequence,
        const TimedFrame* timing, const BYTE* bytes, UINT32 length) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetTimedMetrics(TimedMetrics* metrics) = 0;
};
inline bool qpc_now(UINT64& time) noexcept {
    LARGE_INTEGER ticks{}, frequency{};
    if (!QueryPerformanceCounter(&ticks) || !QueryPerformanceFrequency(&frequency) || ticks.QuadPart < 0 ||
        frequency.QuadPart <= 0 || frequency.QuadPart > 1000000000LL) return false;
    const auto t = UINT64(ticks.QuadPart), f = UINT64(frequency.QuadPart);
    if (t / f > (UINT64(LLONG_MAX) - 10000000) / 10000000) return false;
    time = (t / f) * 10000000 + (t % f) * 10000000 / f;
    return true;
}
inline bool valid_timing(const TimedFrame& t, UINT64 now) noexcept {
    if (t.reserved || t.flags > TimedNeutral || !t.presentation_qpc ||
        t.presentation_qpc > UINT64(LLONG_MAX) - UINT64(FrameDuration) ||
        (t.presentation_qpc > now && t.presentation_qpc - now > TimedHorizon) ||
        (now >= t.presentation_qpc && now - t.presentation_qpc >= UINT64(FrameDuration))) return false;
    if (t.flags == TimedNeutral) return t.frame_id == 0 && t.capture_qpc == 0;
    return t.frame_id && t.capture_qpc && t.capture_qpc <= now && t.capture_qpc <= t.presentation_qpc &&
        now - t.capture_qpc <= TimedMaxAge && t.presentation_qpc - t.capture_qpc <= TimedMaxAge;
}
}
