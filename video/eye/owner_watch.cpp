#include "owner_watch.h"
namespace tingxian::eye {
void OwnerWatch::cancel() noexcept { cancelled_.store(true); session_.stop(); }
bool OwnerWatch::cancelled() noexcept {
    if (cancelled_.load()) return true;
    // A signalled kernel event wins even when the watcher has not been scheduled.
    // Call at frame admission and after a vendor call, not just on the watcher.
    if (!owner_ || !stop_ || WaitForSingleObject(stop_, 0) != WAIT_TIMEOUT
        || WaitForSingleObject(owner_, 0) != WAIT_TIMEOUT) cancel();
    return cancelled_.load();
}
OwnerWatch::~OwnerWatch() {
    close();
    for (HANDLE h : {owner_, stop_, close_}) if (h) CloseHandle(h);
}
bool OwnerWatch::arm(HANDLE owner, HANDLE stop) noexcept {
    if (used_) return false;
    used_ = true;
    if (!owner || owner == INVALID_HANDLE_VALUE || !stop || stop == INVALID_HANDLE_VALUE
        || !DuplicateHandle(GetCurrentProcess(), owner, GetCurrentProcess(), &owner_, SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, 0)
        || !DuplicateHandle(GetCurrentProcess(), stop, GetCurrentProcess(), &stop_, SYNCHRONIZE, FALSE, 0)
        || !GetProcessId(owner_) || GetProcessId(owner_) == GetCurrentProcessId()) { cancel(); return false; }
    close_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!close_) { cancel(); return false; }
    const auto os = WaitForSingleObject(owner_, 0), ss = WaitForSingleObject(stop_, 0);
    if (os != WAIT_TIMEOUT || ss != WAIT_TIMEOUT) { cancel(); return os == WAIT_OBJECT_0 || ss == WAIT_OBJECT_0; }
    try {
        thread_ = std::thread([this] {
            HANDLE handles[] = {stop_, owner_, close_};
            const auto result = WaitForMultipleObjects(3, handles, FALSE, INFINITE);
            if (result != WAIT_OBJECT_0 + 2) cancel();
        });
    } catch (...) { cancel(); return false; }
    return true;
}
void OwnerWatch::close() noexcept {
    if (close_) SetEvent(close_);
    if (thread_.joinable()) thread_.join();
    session_.stop();
}
}
