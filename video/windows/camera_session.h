#pragma once
#include "frame_transport.h"
#include <atomic>
#include <mutex>

namespace tingxian::video {
// Main controller/producer authority. revoke must be idempotent and thread safe:
// close its FramePublisher and stop input, without waiting for device OS calls.
struct OutputLease {
    virtual ~OutputLease() = default;
    virtual void revoke() noexcept = 0;
};
// Injectable OS boundary; prepare does not enable device enumeration. shutdown
// attempts all cleanup; on failure it retains acquired handles for explicit retry.
struct CameraBackend {
    virtual ~CameraBackend() = default;
    virtual HRESULT prepare(const VideoPipeEndpoint& endpoint) noexcept = 0;
    virtual HRESULT start() noexcept = 0;
    virtual HRESULT stop() noexcept = 0;
    virtual HRESULT shutdown() noexcept = 0;
};
enum class CameraSessionState : UINT32 { idle, starting, available, stopping, fault };
struct CameraSessionStatus {
    CameraSessionState state = CameraSessionState::idle;
    HRESULT operation_error = S_OK;
    HRESULT cleanup_error = S_OK;
};
// Device lifetime only: available means Windows Start succeeded, not that a
// meeting received pixels. OS methods run on a dedicated MTA, not the UI thread.
class CameraSession {
    CameraBackend& backend_;
    OutputLease& lease_;
    std::mutex lifecycle_;
    std::atomic<UINT64> cancellation_{0};
    std::atomic<CameraSessionState> state_{CameraSessionState::idle};
    std::atomic<HRESULT> operation_error_{S_OK};
    std::atomic<HRESULT> cleanup_error_{S_OK};
    bool resources_ = false;
    HRESULT cleanup_locked() noexcept;
public:
    CameraSession(CameraBackend& backend, OutputLease& lease) : backend_(backend), lease_(lease) {}
    ~CameraSession();
    CameraSession(const CameraSession&) = delete;
    CameraSession& operator=(const CameraSession&) = delete;
    HRESULT start(const VideoPipeEndpoint& endpoint) noexcept;
    // May run on any thread, including while Windows Start is blocked.
    void request_stop() noexcept;
    HRESULT stop() noexcept;
    CameraSessionStatus status() const noexcept {
        return {state_.load(), operation_error_.load(), cleanup_error_.load()};
    }
};
// Read-only exports/registry inspection; no COM activation or privacy request.
struct CameraProbe {
    bool api_exports_present = false;
    bool machine_registration_present = false;
    bool user_registration_conflict = false;
    HRESULT api_error = S_OK;
    HRESULT registration_error = S_OK;
};
CameraProbe probe_virtual_camera() noexcept;
// Process-lifetime reservation for this user's one device, across login sessions.
// Caller closes a successful handle; it must not wait on or release this mutex.
HRESULT reserve_video_output(HANDLE* handle) noexcept;
// Constructor has no side effects. Destroy CameraSession before backend/lease.
CameraBackend* make_windows_camera_backend() noexcept;
}
