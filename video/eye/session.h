#pragma once
#include "provider.h"
#include <deque>
#include <mutex>

namespace tingxian::eye {
enum class State { off, preparing, ready, stopping, unavailable, fault };
struct Snapshot {
    State state = State::off;
    Error error = Error::none;
    uint64_t generation = 0, accepted = 0, rejected = 0, dropped = 0, delivered = 0;
    size_t queued = 0;
    bool busy = false;
    bool cleanup_uncertain = false;
};
class Session {
    EyeCorrectionProvider& provider_;
    OutputSink& sink_;
    const Clock& clock_;
    mutable std::mutex mutex_;
    Snapshot status_;
    Format format_;
    std::deque<Frame> queue_;
    uint64_t last_id_ = 0, last_time_ = 0;
    bool prepared_ = false;
    bool fresh(uint64_t at) const noexcept;
    void discard() noexcept;
    void finish_cleanup(uint64_t generation, State terminal, Error error) noexcept;
public:
    static constexpr size_t MaxPending = 2;
    static constexpr uint64_t MaxAgeMs = 250;
    Session(EyeCorrectionProvider& provider, OutputSink& sink, const Clock& clock)
        : provider_(provider), sink_(sink), clock_(clock) {}
    // Caller drains stop and joins the execution lane before destroying Session.
    uint64_t begin(Format format); // zero means refused; explicit user admission
    bool prepare(uint64_t generation); // worker lane, queued starts carry a ticket
    bool submit(Frame frame);
    bool process_one(); // worker lane; at most one in flight + two waiting
    void stop() noexcept; // immediate gate + output revocation; never waits on GPU
    void drain_stop() noexcept; // worker lane cleanup after stop
    Snapshot snapshot() const;
};
}
