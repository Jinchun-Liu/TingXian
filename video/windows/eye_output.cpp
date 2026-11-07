#include "eye_output.h"
#include <algorithm>
#include <cmath>

namespace tingxian::video {
namespace {
constexpr size_t BgrBytes = size_t(Width) * Height * 3;
constexpr HRESULT Cancelled = HRESULT_FROM_WIN32(ERROR_CANCELLED);
BYTE rounded(double value) noexcept {
    return static_cast<BYTE>((std::clamp)(std::floor(value + 0.5), 0.0, 255.0));
}
}
bool EyePipeOutput::fresh(const eye::Frame& frame) const noexcept {
    if (domain_ == eye::ClockDomain::tick_ms) {
        const auto now = GetTickCount64();
        return !frame.capture_qpc && !frame.completed_qpc && frame.captured_ms <= now && now - frame.captured_ms <= MaxFrameAgeMs;
    }
    UINT64 now = 0;
    return !frame.captured_ms && frame.capture_qpc && qpc_now(now) && frame.capture_qpc <= frame.completed_qpc &&
        frame.completed_qpc <= now && now - frame.capture_qpc <= TimedMaxAge;
}
bool corrected_bgr_to_nv12(const BYTE* bgr, size_t bytes, BYTE* nv12, size_t capacity) noexcept {
    if (!bgr || !nv12 || bytes != BgrBytes || capacity != FrameBytes) return false;
    // Microsoft BT.709 equations, computer RGB Z=0/S=255. UV is averaged before
    // quantization, centered over each 2x2 block; output type declares this siting.
    for (UINT32 y = 0; y < Height; y += 2) for (UINT32 x = 0; x < Width; x += 2) {
        double sum_b = 0, sum_r = 0, sum_l = 0;
        for (UINT32 row = 0; row < 2; ++row) for (UINT32 col = 0; col < 2; ++col) {
            const auto pos = size_t(y + row) * Width + x + col;
            const auto* p = bgr + pos * 3;
            const double luma = 0.2126 * p[2] + 0.7152 * p[1] + 0.0722 * p[0];
            nv12[pos] = rounded(16.0 + 219.0 * luma / 255.0);
            sum_b += p[0]; sum_r += p[2]; sum_l += luma;
        }
        const auto uv = size_t(Width) * Height + size_t(y / 2) * Width + x;
        nv12[uv] = rounded(128.0 + 112.0 * (sum_b - sum_l) / (4 * 0.9278 * 255));
        nv12[uv + 1] = rounded(128.0 + 112.0 * (sum_r - sum_l) / (4 * 0.7874 * 255));
    }
    return true;
}
void EyePipeOutput::discard_locked() noexcept {
    if (pending_) { ++status_.dropped; pending_.reset(); }
}
void EyePipeOutput::revoke_locked() noexcept {
    ++intent_;
    discard_locked();
    if (status_.state != EyeOutputState::closed && status_.state != EyeOutputState::stopped
        && status_.state != EyeOutputState::fault) status_.state = EyeOutputState::stopping;
    // Event only: even accept or a blocked pipe write cannot hold Session's lock.
    publisher_.cancel();
    changed_.notify_all();
}
void EyePipeOutput::request_stop() noexcept {
    std::lock_guard guard(mutex_);
    revoke_locked();
}
void EyePipeOutput::neutral(uint64_t generation) noexcept {
    std::lock_guard guard(mutex_);
    if (generation >= status_.generation) revoke_locked();
}
HRESULT EyePipeOutput::open_if_current(UINT64 generation, UINT64 ticket, VideoPipeEndpoint* endpoint) noexcept {
    if (!endpoint) return E_POINTER;
    SecureZeroMemory(endpoint, sizeof(*endpoint));
    std::lock_guard lifecycle(lifecycle_);
    {
        std::lock_guard guard(mutex_);
        if (ticket != intent_) return Cancelled;
        if (status_.state != EyeOutputState::closed || sender_.joinable()) return HRESULT_FROM_WIN32(ERROR_BUSY);
        if (!generation || generation <= last_generation_) return E_INVALIDARG;
        last_generation_ = generation;
        status_.generation = generation;
        status_.state = EyeOutputState::opening;
        status_.error = S_OK;
        last_id_ = last_time_ = 0;
    }
    VideoPipeEndpoint candidate{};
    HRESULT hr = domain_ == eye::ClockDomain::qpc_100ns ? publisher_.open_corrected(&candidate) : publisher_.open(&candidate);
    {
        std::lock_guard guard(mutex_);
        if (ticket != intent_) hr = Cancelled;
        if (SUCCEEDED(hr)) {
            status_.state = EyeOutputState::waiting;
            try { sender_ = std::thread(&EyePipeOutput::run, this, ticket, generation); }
            catch (...) { hr = E_OUTOFMEMORY; }
        }
        if (FAILED(hr)) {
            status_.state = hr == Cancelled ? EyeOutputState::stopped : EyeOutputState::fault;
            status_.error = hr;
        } else *endpoint = candidate;
    }
    SecureZeroMemory(&candidate, sizeof(candidate));
    if (FAILED(hr)) publisher_.close();
    return hr;
}
bool EyePipeOutput::corrected(const eye::Frame& frame) noexcept {
    return enqueue(frame.generation, frame.id, frame.captured_ms, frame.bgr.data(), frame.bgr.size(), true, frame.capture_qpc, frame.completed_qpc);
}
bool EyePipeOutput::corrected_nv12(UINT64 generation, UINT64 id, UINT64 captured, const BYTE* data, size_t bytes) noexcept {
    return enqueue(generation, id, captured, data, bytes, false);
}
bool EyePipeOutput::enqueue(UINT64 generation, UINT64 id, UINT64 captured, const BYTE* data, size_t bytes, bool bgr,
                           UINT64 capture_qpc, UINT64 completed_qpc) noexcept {
    std::lock_guard guard(mutex_);
    eye::Frame metadata{generation, id, captured, {}, capture_qpc, completed_qpc};
    const auto time = metadata.captured(domain_);
    if ((status_.state != EyeOutputState::waiting && status_.state != EyeOutputState::streaming)
        || generation != status_.generation || !id || id <= last_id_
        || time < last_time_ || !fresh(metadata) || !data || bytes != (bgr ? BgrBytes : FrameBytes)) {
        ++status_.rejected;
        return false;
    }
    try {
        // Copy only a completed provider result; neither a raw fallback nor an
        // unbounded queue. Allocate before replacing to preserve clear counts.
        Pending next; next.frame = std::move(metadata); next.frame.bgr.assign(data, data + bytes); next.convert_bgr = bgr;
        discard_locked();
        pending_ = std::move(next);
        ++status_.accepted;
        last_id_ = id; last_time_ = time;
    } catch (...) {
        ++status_.rejected;
        status_.error = E_OUTOFMEMORY; status_.state = EyeOutputState::fault;
        revoke_locked();
        return false;
    }
    changed_.notify_one();
    return true;
}
void EyePipeOutput::run(UINT64 ticket, UINT64 generation) noexcept {
    HRESULT hr = publisher_.accept();
    if (SUCCEEDED(hr)) hr = publisher_.begin(generation);
    try {
        std::vector<BYTE> nv12(FrameBytes);
        {
            std::lock_guard guard(mutex_);
            if (SUCCEEDED(hr) && ticket == intent_ && status_.state == EyeOutputState::waiting)
                status_.state = EyeOutputState::streaming;
        }
        while (SUCCEEDED(hr)) {
            Pending pending;
            {
                std::unique_lock lock(mutex_);
                changed_.wait(lock, [&] { return ticket != intent_ || pending_.has_value(); });
                if (ticket != intent_ || status_.state != EyeOutputState::streaming) break;
                pending = std::move(*pending_); pending_.reset();
                status_.busy = true;
            }
            const auto& frame = pending.frame;
            const bool converted = fresh(frame) && (!pending.convert_bgr
                || corrected_bgr_to_nv12(frame.bgr.data(), frame.bgr.size(), nv12.data(), nv12.size()));
            bool send = false;
            {
                std::lock_guard guard(mutex_);
                send = converted && fresh(frame) && ticket == intent_
                    && status_.state == EyeOutputState::streaming;
                if (!send) { ++status_.dropped; status_.busy = false; }
            }
            if (!send) continue;
            // cancel() cannot be undone by a stale conversion completion. Never
            // replace captured_ms with conversion/send time to renew a stale face.
            const auto* pixels = pending.convert_bgr ? nv12.data() : frame.bgr.data();
            hr = domain_ == eye::ClockDomain::qpc_100ns
                ? publisher_.submit_corrected(generation, frame.id, {frame.capture_qpc, frame.completed_qpc}, pixels, FrameBytes)
                : publisher_.submit(generation, frame.id, frame.captured_ms, pixels, FrameBytes);
            {
                std::lock_guard guard(mutex_);
                if (SUCCEEDED(hr)) ++status_.submitted;
                else ++status_.dropped;
                status_.busy = false;
            }
        }
        SecureZeroMemory(nv12.data(), nv12.size());
    } catch (...) { hr = E_OUTOFMEMORY; }
    publisher_.close();
    {
        std::lock_guard guard(mutex_);
        discard_locked(); status_.busy = false;
        if (status_.state != EyeOutputState::fault) {
            if (ticket != intent_) status_.state = EyeOutputState::stopped;
            else { status_.state = EyeOutputState::fault; status_.error = FAILED(hr) ? hr : E_UNEXPECTED; }
        }
    }
    changed_.notify_all();
}
void EyePipeOutput::close() noexcept {
    request_stop();
    std::lock_guard lifecycle(lifecycle_);
    if (sender_.joinable()) sender_.join();
    publisher_.close();
    std::lock_guard guard(mutex_);
    discard_locked(); status_.busy = false; status_.state = EyeOutputState::closed;
}
EyeOutputStatus EyePipeOutput::status() const noexcept {
    std::lock_guard guard(mutex_);
    auto copy = status_; copy.queued = pending_ ? 1 : 0; return copy;
}
}
