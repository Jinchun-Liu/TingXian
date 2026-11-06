#include "worker_owner.h"

namespace tingxian::video {
namespace {
constexpr HRESULT Cancelled = HRESULT_FROM_WIN32(ERROR_CANCELLED);
HRESULT duplicate(HANDLE from, DWORD access, HANDLE* result) noexcept {
    if (!from || from == INVALID_HANDLE_VALUE) return E_INVALIDARG;
    return DuplicateHandle(GetCurrentProcess(), from, GetCurrentProcess(), result,
        access, FALSE, 0) ? S_OK : HRESULT_FROM_WIN32(GetLastError());
}
}
WorkerOwnerGuard::~WorkerOwnerGuard() { close(); release_handles(); }
void WorkerOwnerGuard::release_handles() noexcept {
    for (auto p : {&owner_, &stop_, &close_, &revoked_}) {
        if (*p) CloseHandle(*p);
        *p = nullptr;
    }
}
void WorkerOwnerGuard::revoke(OwnerStopReason reason) noexcept {
    auto expected = OwnerStopReason::none;
    if (reason_.compare_exchange_strong(expected, reason)) {
        session_.request_stop();
        if (revoked_) SetEvent(revoked_);
    }
}
HRESULT WorkerOwnerGuard::arm(HANDLE owner_process, HANDLE stop_event) noexcept {
    if (used_) return HRESULT_FROM_WIN32(ERROR_BUSY);
    used_ = true;
    // Capture before starting the watcher, never after a possible revocation.
    ticket_ = session_.operation_ticket();
    HRESULT hr = duplicate(owner_process, SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, &owner_);
    if (SUCCEEDED(hr)) {
        const DWORD pid = GetProcessId(owner_);
        if (!pid || pid == GetCurrentProcessId()) hr = E_INVALIDARG;
    }
    if (SUCCEEDED(hr)) hr = duplicate(stop_event, SYNCHRONIZE, &stop_);
    if (SUCCEEDED(hr)) {
        close_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        revoked_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (!close_ || !revoked_) hr = HRESULT_FROM_WIN32(GetLastError());
    }
    if (SUCCEEDED(hr)) {
        const DWORD owner = WaitForSingleObject(owner_, 0);
        const DWORD stop = WaitForSingleObject(stop_, 0);
        if (owner == WAIT_OBJECT_0) { revoke(OwnerStopReason::owner_exited); hr = Cancelled; }
        else if (stop == WAIT_OBJECT_0) { revoke(OwnerStopReason::explicit_stop); hr = Cancelled; }
        else if (owner != WAIT_TIMEOUT || stop != WAIT_TIMEOUT) hr = E_INVALIDARG;
    }
    if (FAILED(hr)) {
        revoke(OwnerStopReason::wait_failed);
        release_handles(); return hr;
    }
    try {
        watcher_ = std::thread([this] {
            HANDLE handles[] = {stop_, owner_, close_};
            const DWORD wait = WaitForMultipleObjects(3, handles, FALSE, INFINITE);
            revoke(wait == WAIT_OBJECT_0 ? OwnerStopReason::explicit_stop
                : wait == WAIT_OBJECT_0 + 1 ? OwnerStopReason::owner_exited
                : wait == WAIT_OBJECT_0 + 2 ? OwnerStopReason::guard_closed : OwnerStopReason::wait_failed);
        });
    } catch (...) {
        revoke(OwnerStopReason::wait_failed); release_handles(); return E_OUTOFMEMORY;
    }
    armed_ = true; return S_OK;
}
HRESULT WorkerOwnerGuard::start(const VideoPipeEndpoint& endpoint) noexcept {
    if (!armed_) return E_UNEXPECTED;
    if (started_) return HRESULT_FROM_WIN32(ERROR_BUSY);
    started_ = true;
    // Catch already-signaled handles even if the watcher has not been scheduled.
    const DWORD stop = WaitForSingleObject(stop_, 0), owner = WaitForSingleObject(owner_, 0);
    if (stop == WAIT_OBJECT_0) revoke(OwnerStopReason::explicit_stop);
    else if (owner == WAIT_OBJECT_0) revoke(OwnerStopReason::owner_exited);
    else if (stop != WAIT_TIMEOUT || owner != WAIT_TIMEOUT) revoke(OwnerStopReason::wait_failed);
    if (reason() != OwnerStopReason::none) return Cancelled;
    return session_.start_if_current(endpoint, ticket_);
}
void WorkerOwnerGuard::close() noexcept {
    // Never just detach a live worker from its owner. Closing consumes this lease.
    used_ = true;
    revoke(OwnerStopReason::guard_closed);
    if (close_) SetEvent(close_);
    if (watcher_.joinable()) watcher_.join();
    armed_ = false;
}
}
