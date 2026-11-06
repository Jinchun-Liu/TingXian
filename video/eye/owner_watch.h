#pragma once
#include "session.h"
#include <windows.h>
#include <atomic>
#include <thread>
namespace tingxian::eye {
// Cooperative native stop, paired with the parent's kill-on-close Job for hangs.
// Only inherited restricted handles; never reopen a PID or use named events.
class OwnerWatch {
    Session& session_;
    HANDLE owner_ = nullptr, stop_ = nullptr, close_ = nullptr;
    std::thread thread_;
    std::atomic<bool> cancelled_{false};
    bool used_ = false;
    void cancel() noexcept;
public:
    explicit OwnerWatch(Session& session) : session_(session) {}
    ~OwnerWatch();
    bool arm(HANDLE owner, HANDLE stop) noexcept;
    void close() noexcept;
    bool cancelled() noexcept;
};
}
