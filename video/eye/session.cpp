#include "session.h"
#include <limits>
#include <utility>

namespace tingxian::eye {
bool Session::fresh(uint64_t at) const noexcept {
    const auto now = clock_.now_ms();
    return at <= now && now - at <= MaxAgeMs;
}
void Session::discard() noexcept {
    status_.dropped += queue_.size();
    queue_.clear();
}
uint64_t Session::begin(Format format) {
    std::lock_guard guard(mutex_);
    if (!format.bytes() || status_.state != State::off || status_.busy || prepared_ || status_.cleanup_uncertain
        || status_.generation == (std::numeric_limits<uint64_t>::max)()) return 0;
    ++status_.generation;
    format_ = format;
    status_.state = State::preparing;
    status_.error = Error::none;
    last_id_ = last_time_ = 0;
    sink_.neutral(status_.generation);
    return status_.generation;
}
void Session::finish_cleanup(uint64_t generation, State terminal, Error error) noexcept {
    const bool cleaned = provider_.stop(); // no SDK access on stop's calling thread
    std::lock_guard guard(mutex_);
    prepared_ = status_.busy = false;
    status_.state = status_.generation == generation ? terminal : State::off;
    status_.error = status_.generation == generation ? error : Error::none;
    if (!cleaned) {
        status_.cleanup_uncertain = true;
        status_.state = State::fault;
        status_.error = Error::cleanup;
    }
}
bool Session::prepare(uint64_t generation) {
    Format format;
    {
        std::lock_guard guard(mutex_);
        if (status_.state != State::preparing || status_.busy || generation != status_.generation) return false;
        status_.busy = true;
        format = format_;
    }
    Error error;
    try { error = provider_.prepare(format); }
    catch (...) { error = Error::exception; }
    {
        std::lock_guard guard(mutex_);
        if (generation == status_.generation && status_.state == State::preparing && error == Error::none) {
            prepared_ = true;
            status_.busy = false;
            status_.state = State::ready;
            return true;
        }
        sink_.neutral(status_.generation);
    }
    finish_cleanup(generation, State::unavailable, error);
    return false;
}
bool Session::submit(Frame frame) {
    std::lock_guard guard(mutex_);
    if (status_.state != State::ready || frame.generation != status_.generation || !frame.id
        || frame.id <= last_id_ || frame.captured_ms < last_time_ || !fresh(frame.captured_ms)
        || frame.bgr.size() != format_.bytes()) {
        ++status_.rejected;
        return false;
    }
    while (!queue_.empty() && (!fresh(queue_.front().captured_ms) || queue_.size() >= MaxPending)) {
        queue_.pop_front();
        ++status_.dropped;
    }
    last_id_ = frame.id;
    last_time_ = frame.captured_ms;
    queue_.push_back(std::move(frame));
    ++status_.accepted;
    return true;
}
bool Session::process_one() {
    Frame frame;
    {
        std::lock_guard guard(mutex_);
        if (status_.state != State::ready || status_.busy) return false;
        while (!queue_.empty() && !fresh(queue_.front().captured_ms)) {
            queue_.pop_front();
            ++status_.dropped;
        }
        if (queue_.empty()) return false;
        frame = std::move(queue_.front());
        queue_.pop_front();
        status_.busy = true;
    }
    Result result;
    try { result = provider_.process(frame); }
    catch (...) { result.error = Error::exception; }
    const auto generation = frame.generation;
    {
        std::lock_guard guard(mutex_);
        if (generation != status_.generation || status_.state != State::ready) {
            ++status_.dropped;
            // stop already revoked the sink. Cleanup below precedes restart.
        } else if (result.error == Error::none && result.corrected.size() == format_.bytes()) {
            status_.busy = false;
            if (!fresh(frame.captured_ms)) {
                ++status_.dropped;
                sink_.neutral(generation);
                return false;
            }
            frame.bgr = std::move(result.corrected); // never use input as fallback
            if (sink_.corrected(frame)) { ++status_.delivered; return true; }
            status_.busy = true;
            result.error = Error::provider_contract;
            discard();
            sink_.neutral(generation);
            ++status_.dropped;
            status_.state = State::fault;
            status_.error = result.error;
        } else {
            if (result.error == Error::none) result.error = Error::provider_contract;
            discard();
            sink_.neutral(generation);
            ++status_.dropped;
            status_.state = State::fault;
            status_.error = result.error;
        }
    }
    finish_cleanup(generation, State::fault, result.error);
    return false;
}
void Session::stop() noexcept {
    std::lock_guard guard(mutex_);
    if (status_.generation != (std::numeric_limits<uint64_t>::max)()) ++status_.generation;
    discard();
    status_.error = Error::none;
    status_.state = status_.busy || prepared_ ? State::stopping : State::off;
    if (status_.cleanup_uncertain) { status_.state = State::fault; status_.error = Error::cleanup; }
    sink_.neutral(status_.generation);
}
void Session::drain_stop() noexcept {
    uint64_t generation;
    {
        std::lock_guard guard(mutex_);
        if (status_.state != State::stopping || status_.busy) return;
        status_.busy = true;
        generation = status_.generation;
    }
    finish_cleanup(generation, State::off, Error::none);
}
Snapshot Session::snapshot() const {
    std::lock_guard guard(mutex_);
    auto copy = status_;
    copy.queued = queue_.size();
    return copy;
}
}
