#pragma once
#include "video_timing.h"
#include <algorithm>
#include <deque>
#include <vector>

namespace tingxian::video {
// Access only under FrameMailbox's generation lock. Four queued slots plus one
// current slot; no device, clock or thread is opened here. All times are QPC hns.
class TimedMailbox {
    struct Slot {
        UINT64 sequence = 0;
        TimedFrame timing;
        std::vector<BYTE> pixels;
        bool delivered = false;
        void erase() { std::fill(pixels.begin(), pixels.end(), BYTE{0}); pixels.clear(); sequence = 0; }
    };
    std::deque<Slot> queue_;
    Slot current_;
    UINT64 sequence_ = 0, presentation_ = 0, frame_ = 0, capture_ = 0;
    UINT64 last_delivered_frame_ = 0;
    TimedMetrics metrics_;
    static bool expired(const Slot& slot, UINT64 now) {
        return (now >= slot.timing.presentation_qpc && now - slot.timing.presentation_qpc >= UINT64(FrameDuration)) ||
            (slot.timing.capture_qpc && now >= slot.timing.capture_qpc && now - slot.timing.capture_qpc > TimedMaxAge);
    }
    void discard_current(bool late) {
        if (current_.sequence && !current_.delivered) {
            if (late) ++metrics_.late; else ++metrics_.superseded;
        }
        current_.erase();
    }
public:
    bool mode = false;
    TimedMetrics metrics() const { auto result = metrics_; result.queued = UINT32(queue_.size()); return result; }
    void reject() { ++metrics_.rejected; }
    bool newer_epoch(UINT64 epoch) const { return epoch && epoch > metrics_.epoch; }
    void clear() {
        metrics_.cleared += queue_.size() + UINT64(current_.sequence && !current_.delivered);
        for (auto& slot : queue_) slot.erase();
        queue_.clear(); current_.erase();
        sequence_ = presentation_ = frame_ = capture_ = last_delivered_frame_ = 0;
    }
    void begin(UINT64 epoch) { clear(); mode = true; metrics_.epoch = epoch; }
    HRESULT submit(UINT64 sequence, const TimedFrame& t, const BYTE* data, UINT32 size, UINT64 now) {
        const bool neutral = t.flags == TimedNeutral;
        if (!valid_timing(t, now) || !sequence || sequence <= sequence_ || t.presentation_qpc <= presentation_ ||
            (neutral ? (data || size) : (!data || size != FrameBytes)) ||
            (!neutral && (t.frame_id < frame_ || t.capture_qpc < capture_ ||
                (t.frame_id == frame_ && t.capture_qpc != capture_)))) {
            reject(); return E_INVALIDARG;
        }
        // Expired pending slots cannot fill the queue forever when requests stop.
        while (!queue_.empty() && expired(queue_.front(), now)) {
            ++metrics_.late; queue_.front().erase(); queue_.pop_front();
        }
        if (queue_.size() == TimedQueueCapacity) { reject(); return HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW); }
        try {
            Slot slot; slot.sequence = sequence; slot.timing = t;
            if (!neutral) slot.pixels.assign(data, data + size);
            queue_.push_back(std::move(slot));
        } catch (const std::bad_alloc&) { reject(); return E_OUTOFMEMORY; }
        sequence_ = sequence; presentation_ = t.presentation_qpc;
        if (!neutral) { frame_ = t.frame_id; capture_ = t.capture_qpc; }
        ++metrics_.accepted; return S_OK;
    }
    // Select only due frames. A missed plan is dropped, never relabelled as new.
    const BYTE* select(UINT64 now, LONGLONG previous, TimedSample& stamp, LONGLONG& pts) {
        stamp = {}; stamp.epoch = metrics_.epoch; stamp.delivered_qpc = now;
        pts = (std::max)(LONGLONG(now), previous + 1);
        while (!queue_.empty() && queue_.front().timing.presentation_qpc <= now) {
            auto& front = queue_.front();
            if (expired(front, now) || front.timing.presentation_qpc <= UINT64(previous)) {
                ++metrics_.late; front.erase(); queue_.pop_front(); continue;
            }
            discard_current(false);
            current_ = std::move(front); queue_.pop_front();
        }
        if (current_.sequence && (expired(current_, now) ||
            (!current_.delivered && current_.timing.presentation_qpc <= UINT64(previous)))) discard_current(true);
        if (!current_.sequence) return nullptr;
        const auto& t = current_.timing;
        stamp.sequence = current_.sequence; stamp.frame_id = t.frame_id;
        stamp.capture_qpc = t.capture_qpc; stamp.presentation_qpc = t.presentation_qpc;
        stamp.flags = t.flags | ((current_.delivered || (t.frame_id && t.frame_id == last_delivered_frame_)) ? TimedRepeated : 0);
        if (!current_.delivered) pts = LONGLONG(t.presentation_qpc);
        return t.flags == TimedNeutral ? nullptr : current_.pixels.data();
    }
    void sent(const TimedSample& stamp) {
        if (stamp.flags & TimedNeutral) ++metrics_.neutral;
        else if (stamp.flags & TimedRepeated) ++metrics_.repeated;
        else ++metrics_.unique;
        if (stamp.frame_id) last_delivered_frame_ = stamp.frame_id;
        if (stamp.sequence == current_.sequence) current_.delivered = true;
    }
};
}
