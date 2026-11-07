#include "eye_output.h"
#include <algorithm>
#include <cmath>

namespace tingxian::video {
namespace {
constexpr size_t BgrBytes = size_t(Width) * Height * 3;
constexpr HRESULT Cancelled = HRESULT_FROM_WIN32(ERROR_CANCELLED);
bool fresh(UINT64 captured) noexcept {
    const auto now = GetTickCount64();
    return captured <= now && now - captured <= MaxFrameAgeMs;
}
BYTE rounded(double value) noexcept {
    return static_cast<BYTE>((std::clamp)(std::floor(value + 0.5), 0.0, 255.0));
}
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
    HRESULT hr = publisher_.open(&candidate);
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
    std::lock_guard guard(mutex_);
    if ((status_.state != EyeOutputState::waiting && status_.state != EyeOutputState::streaming)
        || frame.generation != status_.generation || !frame.id || frame.id <= last_id_
        || frame.captured_ms < last_time_ || !fresh(frame.captured_ms) || frame.bgr.size() != BgrBytes) {
        ++status_.rejected;
        return false;
    }
    try {
        // Copy only a completed provider result; neither a raw fallback nor an
        // unbounded queue. Allocate before replacing to preserve clear counts.
        eye::Frame next = frame;
        discard_locked();
        pending_ = std::move(next);
        ++status_.accepted;
        last_id_ = frame.id; last_time_ = frame.captured_ms;
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
            eye::Frame frame;
            {
                std::unique_lock lock(mutex_);
                changed_.wait(lock, [&] { return ticket != intent_ || pending_.has_value(); });
                if (ticket != intent_ || status_.state != EyeOutputState::streaming) break;
                frame = std::move(*pending_); pending_.reset();
                status_.busy = true;
            }
            const bool converted = fresh(frame.captured_ms)
                && corrected_bgr_to_nv12(frame.bgr.data(), frame.bgr.size(), nv12.data(), nv12.size());
            bool send = false;
            {
                std::lock_guard guard(mutex_);
                send = converted && fresh(frame.captured_ms) && ticket == intent_
                    && status_.state == EyeOutputState::streaming;
                if (!send) { ++status_.dropped; status_.busy = false; }
            }
            if (!send) continue;
            // cancel() cannot be undone by a stale conversion completion. Never
            // replace captured_ms with conversion/send time to renew a stale face.
            hr = publisher_.submit(generation, frame.id, frame.captured_ms, nv12.data(), FrameBytes);
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
