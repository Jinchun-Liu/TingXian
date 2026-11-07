#pragma once
#include "frame_transport.h"
#include <condition_variable>
#include <optional>

namespace tingxian::video {
enum class TimedOutputPhase : UINT32 { idle, waiting, ready, reserved, sending, receipt, stopping, stopped, fault };
enum class Admission : UINT32 { pending, admitted, rejected, unknown, cancelled };
struct TimedTicket { UINT64 owner = 0, generation = 0, serial = 0; };
struct TimedReceipt {
    UINT32 version = 2;
    Admission outcome = Admission::pending;
    UINT64 generation = 0, serial = 0;
    HRESULT error = S_OK;
    UINT32 reserved = 0;
};
struct TimedOutputStatus {
    UINT32 version = 2;
    TimedOutputPhase phase = TimedOutputPhase::idle;
    UINT64 generation = 0, serial = 0, submitted = 0, admitted = 0, rejected = 0, cancelled = 0;
    UINT32 local_clear = 1, external_drain = 0, pending = 0;
    HRESULT error = S_OK;
    UINT32 reserved[2]{};
};
static_assert(sizeof(TimedTicket) == 24 && sizeof(TimedReceipt) == 32 && sizeof(TimedOutputStatus) == 80);

// One reservation OR one sending/receipt slot, never a latest-frame overwrite.
// Open/reserve/stage/take/stop belong to one host lane. cancel/status are safe
// concurrently. A cancelled instance is terminal; close/destroy cannot prove
// external drain or grant permission to recreate the joint AV owner.
class TimedPipeOutput {
    FramePublisher publisher_;
    mutable std::mutex mutex_;
    std::condition_variable changed_;
    std::thread sender_;
    HANDLE wake_ = nullptr; // auto reset, single owning-lane waiter
    UINT64 identity_ = 0;
    bool revoked_ = false;
    TimedOutputStatus status_;
    std::optional<TimedReceipt> receipt_;
    struct Pending {
        TimedFrame timing;
        std::vector<BYTE> pixels;
        ~Pending() { if(!pixels.empty()) SecureZeroMemory(pixels.data(), pixels.size()); }
    };
    std::unique_ptr<Pending> pending_;
    void run() noexcept;
    void notify() noexcept { SetEvent(wake_); changed_.notify_all(); }
    bool owns(TimedTicket t) const noexcept { return t.owner && t.owner == identity_ && t.generation && t.generation == status_.generation && t.serial && t.serial == status_.serial; }
public:
    TimedPipeOutput();
    ~TimedPipeOutput();
    TimedPipeOutput(const TimedPipeOutput&) = delete;
    TimedPipeOutput& operator=(const TimedPipeOutput&) = delete;
    HRESULT open(UINT64 generation, VideoPipeEndpoint* endpoint) noexcept;
    HRESULT reserve(UINT64 generation, TimedTicket* ticket) noexcept;
    HRESULT stage(TimedTicket ticket, const TimedFrame& timing, const BYTE* bytes, UINT32 size) noexcept;
    HRESULT take(TimedTicket ticket, TimedReceipt* receipt) noexcept;
    void cancel() noexcept;
    void stop() noexcept;
    TimedOutputStatus status() const noexcept;
    HANDLE event() const noexcept { return wake_; } // borrowed, never close
};
}
