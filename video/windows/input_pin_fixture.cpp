#include "input_frame.h"
// CI-only DLL exercising the SAME callback implementation as the production
// component. No camera/source/reader creation API is linked into this fixture.
extern "C" __declspec(dllexport) HRESULT WINAPI TxCameraFixtureCallback(IMFSourceReaderCallback** out) noexcept {
    try { return tingxian::camera_input::make_callback(std::make_shared<tingxian::camera_input::Mailbox>(1),
        LONG(tingxian::camera_input::Width * 4), out); }
    catch (...) { return E_OUTOFMEMORY; }
}
