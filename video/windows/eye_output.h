#pragma once
#include "frame_transport.h"
#include "../eye/provider.h"
#include <condition_variable>
#include <optional>

namespace tingxian::video {
// Full-range, gamma-coded packed BGR8 -> limited-range BT.709, centered 4:2:0.
// Exact sizes, no scaling or timestamp changes; false leaves output untouched.
bool corrected_bgr_to_nv12(const BYTE* bgr, size_t bytes, BYTE* nv12, size_t capacity) noexcept;

enum class EyeOutputState { closed, opening, waiting, streaming, stopping, stopped, fault };
struct EyeOutputStatus {
    EyeOutputState state = EyeOutputState::closed;
    UINT64 generation = 0, accepted = 0, rejected = 0, dropped = 0, submitted = 0;
    size_t queued = 0;
    bool busy = false;
    HRESULT error = S_OK;
};

// No camera, SDK, device registration, UI or disk output. The owning host must
// explicitly open AFTER Session::prepare, carry its pre-dispatch ticket, and
// provide the transient endpoint only to the authorized output/preview owner.
// Session callback never waits for pipe I/O: one latest corrected frame plus one
// sender-owned frame. submitted means pipe handoff, not device/meeting receipt.
class EyePipeOutput final : public eye::OutputSink {
    friend struct EyeOutputTestPeer; // native test observes the actual pipe DACL
    FramePublisher publisher_;
    std::mutex lifecycle_;
    mutable std::mutex mutex_;
    std::condition_variable changed_;
    std::thread sender_;
    struct Pending {
        eye::Frame frame;
        bool convert_bgr = true;
        Pending() = default;
        Pending(Pending&&) = default;
        Pending& operator=(Pending&&) = default;
        ~Pending() { if (!frame.bgr.empty()) SecureZeroMemory(frame.bgr.data(), frame.bgr.size()); }
    };
    std::optional<Pending> pending_;
    EyeOutputStatus status_;
    UINT64 last_generation_ = 0, last_id_ = 0, last_time_ = 0;
    std::atomic<UINT64> intent_{0};
    void discard_locked() noexcept;
    void revoke_locked() noexcept;
    void run(UINT64 ticket, UINT64 generation) noexcept;
    bool enqueue(UINT64 generation, UINT64 id, UINT64 captured, const BYTE* data, size_t bytes, bool bgr) noexcept;
public:
    EyePipeOutput() = default;
    ~EyePipeOutput() override { close(); }
    EyePipeOutput(const EyePipeOutput&) = delete;
    EyePipeOutput& operator=(const EyePipeOutput&) = delete;
    UINT64 operation_ticket() const noexcept { return intent_.load(); }
    HRESULT open_if_current(UINT64 generation, UINT64 ticket, VideoPipeEndpoint* endpoint) noexcept;
    void neutral(uint64_t generation) noexcept override;
    bool corrected(const eye::Frame& frame) noexcept override;
    // Native relay for an already corrected/converted preview frame. No second
    // color conversion and no timestamp refresh. Not a raw camera entry point.
    bool corrected_nv12(UINT64 generation, UINT64 id, UINT64 captured, const BYTE* data, size_t bytes) noexcept;
    void request_stop() noexcept; // fast revoke; never joins the sender
    void close() noexcept; // owning host lane: revoke, join, release, then allow new generation
    EyeOutputStatus status() const noexcept;
};
}
