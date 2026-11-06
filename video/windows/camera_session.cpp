#include "camera_session.h"

namespace tingxian::video {
namespace { constexpr HRESULT Cancelled = HRESULT_FROM_WIN32(ERROR_CANCELLED); }
CameraSession::~CameraSession() { stop(); }

void CameraSession::request_stop() noexcept {
    ++cancellation_;
    auto state = state_.load();
    while ((state == CameraSessionState::starting || state == CameraSessionState::available)
        && !state_.compare_exchange_weak(state, CameraSessionState::stopping)) {}
    // Independent of lifecycle_: a slow Windows Start cannot keep frames flowing.
    lease_.revoke();
}
HRESULT CameraSession::cleanup_locked() noexcept {
    if (!resources_) return S_OK;
    state_ = CameraSessionState::stopping;
    const HRESULT stopped = backend_.stop();
    const HRESULT shutdown = backend_.shutdown(); // also after Stop failure
    resources_ = FAILED(shutdown);
    const HRESULT error = FAILED(stopped) ? stopped : shutdown;
    cleanup_error_ = error;
    state_ = FAILED(error) ? CameraSessionState::fault : CameraSessionState::idle;
    return error;
}
HRESULT CameraSession::start(const VideoPipeEndpoint& endpoint) noexcept {
    return start_if_current(endpoint, operation_ticket());
}
HRESULT CameraSession::start_if_current(const VideoPipeEndpoint& endpoint, UINT64 ticket) noexcept {
    std::lock_guard guard(lifecycle_);
    if (ticket != cancellation_.load()) return Cancelled;
    if (resources_) return HRESULT_FROM_WIN32(ERROR_BUSY);
    if (!valid_video_endpoint(endpoint)) {
        lease_.revoke(); operation_error_ = E_INVALIDARG;
        state_ = CameraSessionState::fault; return E_INVALIDARG;
    }
    operation_error_ = S_OK; cleanup_error_ = S_OK;
    state_ = CameraSessionState::starting;
    resources_ = true; // prepare may partially acquire resources even on failure
    HRESULT hr = backend_.prepare(endpoint);
    if (SUCCEEDED(hr) && ticket != cancellation_.load()) hr = Cancelled;
    if (SUCCEEDED(hr)) hr = backend_.start();
    auto starting = CameraSessionState::starting;
    if (SUCCEEDED(hr) && (ticket != cancellation_.load()
        || !state_.compare_exchange_strong(starting, CameraSessionState::available))) hr = Cancelled;
    if (FAILED(hr)) {
        lease_.revoke(); operation_error_ = hr;
        cleanup_locked(); state_ = CameraSessionState::fault;
    }
    return hr; // original error is distinct from cleanup_error
}
HRESULT CameraSession::stop() noexcept {
    request_stop();
    std::lock_guard guard(lifecycle_);
    const HRESULT hr = cleanup_locked();
    if (SUCCEEDED(hr)) {
        operation_error_ = S_OK; cleanup_error_ = S_OK;
        state_ = CameraSessionState::idle;
    }
    return hr;
}
}
