#pragma once
#include "device_bridge.h"
#include "camera_session.h"
#include <memory>
namespace tingxian::video {
// C++ injection seam, not exported by the production DLL.
void* make_device_bridge(std::unique_ptr<CameraBackend>, TxDeviceRevoke, void*) noexcept;
}
