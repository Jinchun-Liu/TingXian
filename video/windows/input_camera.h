#pragma once
#include "input_frame.h"
#include <string>

namespace tingxian::camera_input {
enum class Phase { idle, preparing, ready, running, stopping, stopped, fault };
struct Status {
    Phase phase = Phase::idle;
    HRESULT operation = S_OK, flush = S_OK, shutdown = S_OK, release = S_OK;
    bool cleanup_uncertain = false;
    Counts frames;
};
// Optional native component, not yet wired to main-window start or shipped in
// the application installer. All methods and destruction on its creating MTA.
// One explicit start per instance; use a NEW generation/instance after stop.
class CameraInput {
    struct Impl;
    std::unique_ptr<Impl> impl_;
public:
    CameraInput();
    ~CameraInput();
    CameraInput(const CameraInput&) = delete;
    CameraInput& operator=(const CameraInput&) = delete;
    HRESULT prepare(const std::wstring& selected_symbolic_link, UINT64 generation, bool explicit_intent) noexcept;
    HRESULT start() noexcept;
    HRESULT read(Frame& out) noexcept; // no wait; S_FALSE = no fresh frame
    HRESULT revoke() noexcept; // gate only, before either input's blocking cleanup
    HRESULT stop() noexcept;
    Status status() const noexcept;
    HANDLE ready_event() const noexcept; // borrowed, never close
};
}
