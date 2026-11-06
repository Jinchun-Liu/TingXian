#pragma once
#include "camera_session.h"

namespace tingxian::video {
enum class OwnerStopReason : UINT32 { none, explicit_stop, owner_exited, guard_closed, wait_failed };
// One explicit user action -> one worker/guard. The main controller passes a
// restricted real process handle and an anonymous manual-reset stop event via
// PROC_THREAD_ATTRIBUTE_HANDLE_LIST. Never accept a PID and reopen it, a named
// event, command text, preferences, or a handle supplied by the display window.
// This is cooperative lifetime control, not a hostile-process security boundary.
// No COM/device operations occur on the watcher. Destroy before CameraSession;
// the caller must explicitly inspect session.stop() on the owning MTA afterward.
class WorkerOwnerGuard {
    CameraSession& session_;
    HANDLE owner_ = nullptr, stop_ = nullptr, close_ = nullptr, revoked_ = nullptr;
    std::thread watcher_;
    std::atomic<OwnerStopReason> reason_{OwnerStopReason::none};
    bool used_ = false, armed_ = false, started_ = false;
    UINT64 ticket_ = 0;
    void revoke(OwnerStopReason reason) noexcept;
    void release_handles() noexcept;
public:
    explicit WorkerOwnerGuard(CameraSession& session) : session_(session) {}
    ~WorkerOwnerGuard();
    WorkerOwnerGuard(const WorkerOwnerGuard&) = delete;
    WorkerOwnerGuard& operator=(const WorkerOwnerGuard&) = delete;
    // arm/start/close are serialized on the worker's owning thread. Failed arm
    // also revokes; this guard can never be armed again or restarted after stop.
    HRESULT arm(HANDLE owner_process, HANDLE stop_event) noexcept;
    HRESULT start(const VideoPipeEndpoint& endpoint) noexcept;
    void close() noexcept;
    OwnerStopReason reason() const noexcept { return reason_.load(); }
    // Borrowed completion event: signaled AFTER producer authority is revoked.
    HANDLE revoked_event() const noexcept { return revoked_; }
};
}
