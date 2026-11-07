#pragma once
#include "video_contract.h"
#include "timed_mailbox.h"
#include <algorithm>
#include <limits>
#include <mutex>
#include <vector>

namespace tingxian::video {
// One latest frame, independent of the Media Foundation sample allocator pool.
// A lease expires using the output process's monotonic clock, not a worker timer.
class FrameMailbox {
    std::mutex lock_;
    std::vector<BYTE> bytes_ = std::vector<BYTE>(FrameBytes);
    FrameMetrics metrics_;
    UINT64 frame_id_ = 0;
    UINT64 captured_at_ = 0;
    UINT64 last_presented_ = 0;
    bool has_frame_ = false;
    TimedMailbox timed_;

    void invalidate() {
        timed_.clear();
        if (has_frame_ && frame_id_ != last_presented_) ++metrics_.dropped;
        has_frame_ = false;
        metrics_.enabled = FALSE;
        if (metrics_.generation != (std::numeric_limits<UINT64>::max)()) ++metrics_.generation;
        std::fill(bytes_.begin(), bytes_.end(), BYTE{0});
    }
public:
    HRESULT begin(UINT64* generation) {
        if (!generation) return E_POINTER;
        std::lock_guard guard(lock_);
        if (metrics_.enabled && timed_.mode) return HRESULT_FROM_WIN32(ERROR_BUSY);
        invalidate();
        if (metrics_.generation == (std::numeric_limits<UINT64>::max)()) return E_UNEXPECTED;
        frame_id_ = last_presented_ = captured_at_ = 0;
        metrics_.enabled = TRUE;
        timed_.mode = false;
        *generation = metrics_.generation;
        return S_OK;
    }
    HRESULT submit(UINT64 generation, UINT64 id, UINT64 captured_at, const BYTE* data, UINT32 size) {
        std::lock_guard guard(lock_);
        const auto now = GetTickCount64();
        if (timed_.mode || !data || size != FrameBytes || !metrics_.enabled || generation != metrics_.generation
            || id == 0 || id <= frame_id_ || captured_at > now || now - captured_at > MaxFrameAgeMs) {
            ++metrics_.rejected;
            return E_INVALIDARG;
        }
        if (has_frame_ && frame_id_ != last_presented_) ++metrics_.dropped;
        std::copy_n(data, size, bytes_.begin());
        frame_id_ = id;
        captured_at_ = captured_at;
        has_frame_ = true;
        ++metrics_.accepted;
        return S_OK;
    }
    void stop() { std::lock_guard guard(lock_); invalidate(); }
    void reject() { std::lock_guard guard(lock_); ++metrics_.rejected; }
    FrameMetrics metrics() { std::lock_guard guard(lock_); return metrics_; }
    TimedMetrics timed_metrics() {
        std::lock_guard guard(lock_);
        auto result = timed_.metrics();
        result.enabled = timed_.mode && metrics_.enabled;
        result.generation = metrics_.generation;
        return result;
    }
    HRESULT begin_timed(UINT64 epoch, UINT64* generation) {
        if (!generation) return E_POINTER;
        std::lock_guard guard(lock_);
        if (!timed_.newer_epoch(epoch)) return E_INVALIDARG;
        if (metrics_.enabled && !timed_.mode) return HRESULT_FROM_WIN32(ERROR_BUSY);
        invalidate();
        if (metrics_.generation == (std::numeric_limits<UINT64>::max)()) return E_UNEXPECTED;
        timed_.begin(epoch); metrics_.enabled = TRUE; *generation = metrics_.generation; return S_OK;
    }
    HRESULT submit_timed(UINT64 generation, UINT64 sequence, const TimedFrame* timing,
        const BYTE* data, UINT32 size, UINT64 now) {
        std::lock_guard guard(lock_);
        if (!timing || !timed_.mode || !metrics_.enabled || generation != metrics_.generation) {
            timed_.reject(); return E_INVALIDARG;
        }
        return timed_.submit(sequence, *timing, data, size, now);
    }

    // The caller owns the allocator's locked NV12 buffer. Padding is cleared too.
    template<class Send>
    HRESULT write(BYTE* scanline, LONG pitch, BYTE* allocation, DWORD length, Send&& send) {
        UINT64 now = 0;
        if (!qpc_now(now)) return E_UNEXPECTED;
        return write_sample(scanline, pitch, allocation, length, now, 0,
            [&](const TimedSample*, LONGLONG) { return send(); });
    }
    template<class Send>
    HRESULT write_sample(BYTE* scanline, LONG pitch, BYTE* allocation, DWORD length,
        UINT64 qpc, LONGLONG previous, Send&& send) {
        if (!qpc || qpc > UINT64(LLONG_MAX) - UINT64(FrameDuration) || previous < 0 || previous == LLONG_MAX)
            return E_INVALIDARG;
        if (!scanline || !allocation || pitch < static_cast<LONG>(Width)) return E_INVALIDARG;
        const auto base = reinterpret_cast<uintptr_t>(allocation);
        const auto start = reinterpret_cast<uintptr_t>(scanline);
        const UINT64 needed = UINT64(pitch) * (Height + Height / 2 - 1) + Width;
        if (start < base || start - base > length || needed > length - (start - base)) return E_INVALIDARG;
        std::lock_guard guard(lock_);
        const auto now = GetTickCount64();
        if (!timed_.mode && has_frame_ && (captured_at_ > now || now - captured_at_ > MaxFrameAgeMs)) invalidate();
        std::fill_n(allocation, length, BYTE{0});
        TimedSample stamp;
        LONGLONG pts = (std::max)(LONGLONG(qpc), previous + 1);
        const BYTE* pixels = metrics_.enabled && has_frame_ ? bytes_.data() : nullptr;
        if (timed_.mode) pixels = timed_.select(qpc, previous, stamp, pts);
        const bool live = pixels != nullptr;
        for (UINT32 y = 0; y < Height + Height / 2; ++y) {
            BYTE* row = scanline + size_t(y) * pitch;
            if (live) std::copy_n(pixels + size_t(y) * Width, Width, row);
            else std::fill_n(row, Width, BYTE(y < Height ? 32 : 128));
        }
        // Hold the generation lock through handoff: StopVideo cannot return and
        // then have an already-rendered old frame queued by a delayed callback.
        const HRESULT sent = send(timed_.mode ? &stamp : nullptr, pts);
        if (FAILED(sent)) return sent;
        if (timed_.mode) { timed_.sent(stamp); return S_OK; }
        if (!live) ++metrics_.neutral_samples;
        else if (last_presented_ == frame_id_) ++metrics_.repeated_samples;
        else { ++metrics_.unique_samples; last_presented_ = frame_id_; }
        return S_OK;
    }
};
}
