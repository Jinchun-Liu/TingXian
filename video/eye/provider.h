#pragma once
#include <cstddef>
#include <cstdint>
#include <vector>

namespace tingxian::eye {
struct Format {
    uint32_t width = 0, height = 0;
    // First contract: packed BGR8, no padding, at most 1080p. No audio.
    size_t bytes() const noexcept {
        return width && height && width <= 1920 && height <= 1080
            ? size_t(width) * height * 3 : 0;
    }
};
enum class ClockDomain { tick_ms, qpc_100ns };
struct Frame {
    uint64_t generation = 0, id = 0, captured_ms = 0;
    std::vector<uint8_t> bgr;
    // Explicit QPC mode only. Legacy captured_ms stays zero in that mode.
    // Completion is assigned by Session after inference, never by input/provider.
    uint64_t capture_qpc = 0, completed_qpc = 0;
    uint64_t captured(ClockDomain domain) const noexcept {
        return domain == ClockDomain::qpc_100ns ? capture_qpc : captured_ms;
    }
};
enum class Error { none, unavailable, invalid_frame, no_face, inference, provider_contract, exception, cleanup };
struct Result {
    Error error = Error::unavailable;
    std::vector<uint8_t> corrected;
};
struct Capabilities {
    const char* provider;
    const char* version;
    bool local_only;
};
// All methods are invoked on ONE worker execution lane, never the UI thread.
// prepare/process may block in a vendor driver: the process supervisor must own
// the timeout/kill boundary. stop must tolerate partial preparation and repeats.
// No provider is constructed or prepared automatically by the application.
class EyeCorrectionProvider {
public:
    virtual ~EyeCorrectionProvider() = default;
    virtual Capabilities capabilities() const noexcept = 0;
    virtual Error prepare(Format) = 0;
    virtual Result process(const Frame&) = 0;
    virtual bool stop() noexcept = 0; // false requires process teardown, not restart
};
// A synchronous handoff, not a claim of device/meeting delivery. Called under
// the session generation lock, so it must be short, noexcept and non-reentrant.
// neutral must invalidate any pending output lease; never retain the last face.
class OutputSink {
public:
    virtual ~OutputSink() = default;
    virtual void neutral(uint64_t generation) noexcept = 0;
    virtual bool corrected(const Frame& frame) noexcept = 0;
};
class Clock {
public:
    virtual ~Clock() = default;
    virtual uint64_t now_ms() const noexcept = 0;
    virtual uint64_t now_qpc() const noexcept { return 0; }
};
}
