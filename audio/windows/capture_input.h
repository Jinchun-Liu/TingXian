#pragma once
#include "audio_output.h"
#include <array>
#include <limits>

namespace tingxian::capture {
constexpr uint32_t rate = 48000, max_frames = 4800;
constexpr uint64_t max_age = 2500000, clock_residual = 10000; // 250 ms / 1 ms, experimental.
struct Selection { std::wstring microphone_id, output_return_id; };
HRESULT validate_selection(const std::vector<audio::Endpoint>&, const Selection&) noexcept;
struct Format { uint32_t rate = 0, channels = 0, buffer_frames = 0; };
struct RawPacket {
    const float* samples = nullptr;
    uint32_t frames = 0, flags = 0;
    uint64_t first_frame = 0, qpc_100ns = 0;
};
struct Packet {
    uint64_t generation = 0, first_frame = 0, qpc_100ns = 0;
    uint32_t frames = 0, flags = 0;
    std::array<float, max_frames> samples{};
    void clear() noexcept { generation = first_frame = qpc_100ns = frames = flags = 0; SecureZeroMemory(samples.data(), sizeof(samples)); }
    ~Packet() { clear(); }
};
struct Cleanup { HRESULT stop_error = S_OK, reset_error = S_OK; bool reset = false; };
// Stop/Reset document S_FALSE for already stopped/reset. Other positive results
// are not silently accepted. Keep raw HRESULTs in Status for diagnostics.
inline bool cleanup_success(HRESULT hr) noexcept { return hr == S_OK || hr == S_FALSE; }
struct Backend {
    virtual ~Backend() = default;
    virtual HRESULT inspect(const Selection&) noexcept = 0; // Metadata only.
    virtual HRESULT prepare(const Selection&, Format&) noexcept = 0;
    virtual HRESULT start() noexcept = 0;
    // S_FALSE is empty. S_OK with frames > 0 ALWAYS requires release(frames),
    // including malformed/late/discontinuous packets that will not be delivered.
    virtual HRESULT acquire(RawPacket&) noexcept = 0;
    virtual HRESULT release(uint32_t frames) noexcept = 0;
    virtual HRESULT now(uint64_t& qpc_100ns) noexcept = 0;
    virtual Cleanup stop_reset() noexcept = 0;
    virtual HANDLE ready_event() const noexcept { return nullptr; }
};
enum class Phase : uint32_t { idle, prepared, running, stopped, fault };
struct Status {
    Phase phase = Phase::idle;
    HRESULT operation_error = S_OK, release_error = S_OK;
    Cleanup cleanup;
    uint64_t delivered_frames = 0, discarded_frames = 0, packets = 0;
    uint32_t startup_discontinuities = 0;
};
// Native owner thread only. No default input, retry-to-another-device, loopback,
// ASR, callback execution, background task, retained PCM or automatic start.
class Session {
    Backend& backend_;
    const DWORD owner_ = GetCurrentThreadId();
    Status status_;
    Format format_;
    uint64_t generation_ = 0, next_frame_ = 0, last_qpc_ = 0;
    uint32_t last_frames_ = 0;
    bool resources_ = false, have_packet_ = false;
    HRESULT fault(HRESULT) noexcept;
public:
    explicit Session(Backend& backend) : backend_(backend) {}
    ~Session();
    Session(const Session&) = delete;
    Session& operator=(const Session&) = delete;
    HRESULT prepare(const Selection&, uint64_t generation, bool explicit_intent) noexcept;
    HRESULT start(uint64_t generation) noexcept;
    HRESULT read(uint64_t generation, Packet&) noexcept;
    HRESULT stop() noexcept;
    Status status() const noexcept { return status_; }
    Format format() const noexcept { return format_; }
};
std::unique_ptr<Backend> make_windows_backend();
}
