#pragma once
#include <windows.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <atomic>
#include <memory>
#include <mutex>
#include <vector>

namespace tingxian::camera_input {
inline constexpr UINT32 Width = 1280, Height = 720, BgrBytes = Width * Height * 3;
inline constexpr UINT64 MaxAge = 2500000; // QPC in 100 ns, not callback/sample-relative time.
struct Frame {
    UINT64 generation = 0, sequence = 0, capture_qpc = 0;
    UINT32 discontinuity = 0;
    std::vector<BYTE> bgr;
    Frame() = default;
    Frame(const Frame&) = delete;
    Frame& operator=(const Frame&) = delete;
    Frame(Frame&& other) noexcept;
    Frame& operator=(Frame&& other) noexcept;
    ~Frame() { clear(); }
    void clear() noexcept;
};
struct Counts {
    UINT64 accepted = 0, delivered = 0, stale = 0, superseded = 0, cleared = 0, late = 0, discontinuities = 0;
    UINT32 queued = 0;
    HRESULT error = S_OK;
    bool closed = false;
};
// RGB32 is B,G,R,unused. No scaling/mirroring/eye or mouth changes here.
// scanline_offset is the top visible row relative to start; signed pitch is
// validated for every row BEFORE any read. On failure out is securely cleared.
HRESULT copy_bgr(const BYTE* start, size_t length, size_t scanline_offset, LONG pitch, Frame& out);
HRESULT decode(IMFSample*, LONG fallback_stride, Frame& out);
// Single sample slot, per immutable generation. Callback never owns the reader,
// invokes another read, opens a device, records frames or publishes raw output.
class Mailbox {
    const UINT64 generation_;
    mutable std::mutex mutex_;
    std::atomic<bool> closed_{false};
    Frame frame_;
    Counts counts_;
    UINT64 previous_ = 0, sequence_ = 0;
    HANDLE ready_ = nullptr, flushed_ = nullptr, retired_ = nullptr;
    void clear_locked() noexcept;
public:
    explicit Mailbox(UINT64 generation);
    ~Mailbox();
    Mailbox(const Mailbox&) = delete;
    bool valid() const noexcept;
    void sample(HRESULT, DWORD stream, DWORD flags, IMFSample*, LONG stride, UINT64 now) noexcept;
    HRESULT take(UINT64 generation, UINT64 now, Frame& out) noexcept;
    void fault(HRESULT) noexcept;
    void close() noexcept; // gate closes before waiting for any copy in progress
    Counts counts() const noexcept;
    HANDLE ready() const noexcept { return ready_; }
    HANDLE flushed() const noexcept { return flushed_; }
    HANDLE retired() const noexcept { return retired_; }
};
HRESULT make_callback(const std::shared_ptr<Mailbox>&, LONG stride, IMFSourceReaderCallback**);
}
