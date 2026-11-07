#pragma once
#include <windows.h>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace tingxian::audio {
constexpr uint32_t sample_rate = 48000;
constexpr uint32_t max_write_frames = 960; // 20 ms; no growing PCM queue here.
constexpr uint32_t max_buffer_frames = 4800; // Explicit experimental ceiling: 100 ms.
enum class Flow { render, capture };
struct Endpoint {
    std::wstring id, name, adapter_id, hardware_id, service;
    Flow flow = Flow::render;
    bool active = false;
    bool topology_resolved = false;
};
struct Pair { std::wstring render_id, capture_id; };
// PnP identity, not friendly names. This does not certify driver signatures or
// license rights, and a capture endpoint is never opened by this library.
bool is_basic_cable(const Endpoint& endpoint) noexcept;
HRESULT validate_pair(const std::vector<Endpoint>& endpoints, const Pair& pair) noexcept;
// Metadata only: no IAudioClient activation/initialization/start.
HRESULT enumerate_endpoints(std::vector<Endpoint>& endpoints) noexcept;
// Exact mono duplication, with no gain, synthesis or clipping. Also used by the
// native writer; tests exercise the same sample transformation without a device.
HRESULT copy_mono_interleaved(const float* mono, uint32_t frames, uint32_t channels,
                              float* output, uint32_t output_samples) noexcept;
struct Format { uint32_t rate = 0, channels = 0, buffer_frames = 0; uint64_t clock_frequency = 0; };
struct Snapshot {
    uint32_t padding = 0;
    uint64_t position = 0, qpc_100ns = 0; // IAudioClock units; NOT sample frames.
};
struct Cleanup {
    HRESULT stop_error = S_OK, reset_error = S_OK;
    bool client_reset = false;
    // Windows backend always returns false: Reset does not prove bridge/receiver drain.
    bool external_drain_verified = false;
};
struct Backend {
    virtual ~Backend() = default;
    virtual HRESULT prepare(const Pair&, uint32_t channels, Format&) noexcept = 0;
    virtual HRESULT start() noexcept = 0;
    virtual HRESULT snapshot(Snapshot&) noexcept = 0;
    virtual HRESULT write_mono(const float*, uint32_t frames, bool silence) noexcept = 0;
    virtual Cleanup stop_reset() noexcept = 0;
    // Borrowed auto-reset event; owner may wait with its cancellation event.
    // No event means this backend is synthetic or not prepared.
    virtual HANDLE ready_event() const noexcept { return nullptr; }
};
// Revoke both media lanes before OS cleanup. Must be nonblocking/noexcept; do not
// call back into Session. Real A/V owner integration is a separate required step.
struct OutputLease { virtual ~OutputLease() = default; virtual void revoke() noexcept = 0; };
enum class Phase { idle, prepared, running, stopped, fault };
enum class Source { self_microphone, system_loopback, remote, output_return };
struct Ticket {
    uint64_t generation = 0, serial = 0, first_frame = 0;
    uint32_t frames = 0;
    Snapshot clock;
};
struct Status {
    Phase phase = Phase::idle;
    HRESULT operation_error = S_OK;
    Cleanup cleanup;
    uint64_t submitted_frames = 0;
};
// All calls, destruction and backend lifetime belong to one MTA owner thread.
// No capture or background thread; no retained PCM; one outstanding reservation.
// Default idle. Source is an internal routing assertion, not media authentication.
class Session {
    Backend& backend_;
    OutputLease& lease_;
    DWORD owner_ = GetCurrentThreadId();
    Status status_;
    Format format_;
    uint64_t generation_ = 0, serial_ = 0;
    Ticket pending_;
    Snapshot last_clock_;
    bool have_clock_ = false, resources_ = false;
    HRESULT fault(HRESULT error) noexcept;
    HRESULT observe(Snapshot& snapshot, uint64_t& consumed_frames) noexcept;
public:
    Session(Backend& backend, OutputLease& lease) : backend_(backend), lease_(lease) {}
    ~Session();
    Session(const Session&) = delete;
    Session& operator=(const Session&) = delete;
    HRESULT prepare(const std::vector<Endpoint>& inventory, const Pair& pair,
                    uint32_t channels, uint64_t generation) noexcept;
    HRESULT start(uint64_t generation, bool explicit_output_intent) noexcept;
    // S_FALSE means backpressure; caller must not discard source PCM or spin.
    HRESULT reserve(uint64_t generation, uint32_t frames, Ticket& ticket) noexcept;
    HRESULT commit(const Ticket&, Source, const float* mono, uint32_t frames) noexcept;
    HRESULT stop() noexcept;
    Status status() const noexcept { return status_; }
    Format format() const noexcept { return format_; }
};
// Construction does not initialize COM or touch audio. prepare re-enumerates and
// validates exact IDs before Activate; it never uses GetDefaultAudioEndpoint.
std::unique_ptr<Backend> make_windows_backend();
}
