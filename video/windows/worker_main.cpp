// Explicit local prerequisite inspection only. No physical camera, device
// registration, GPU/SDK load, credentials, network or media start entry point.
#include "worker_owner.h"
#include <iostream>
#include <memory>
#include <string>
#include <cerrno>
using namespace tingxian::video;
namespace {
HANDLE handle(const wchar_t* text) {
    if (!text || !*text) return nullptr;
    for (const auto* p = text; *p; ++p) if (*p < L'0' || *p > L'9') return nullptr;
    errno = 0; const auto value = _wcstoui64(text, nullptr, 10);
    return errno ? nullptr : reinterpret_cast<HANDLE>(value);
}
struct NoMediaLease : OutputLease { void revoke() noexcept override {} };
}
int wmain(int argc, wchar_t** argv) {
    if (argc != 4 || std::wstring(argv[1]) != L"--inspect-v1") return 2;
    // Future explicit runtime loading must use verified paths; no search in CWD.
    if (!SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_APPLICATION_DIR)) return 3;
    HANDLE owner = handle(argv[2]), stop = handle(argv[3]);
    if (!owner || !stop) return 2;
    std::unique_ptr<CameraBackend> backend(make_windows_camera_backend());
    if (!backend) return 3;
    NoMediaLease lease; CameraSession session(*backend, lease); WorkerOwnerGuard guard(session);
    const HRESULT armed = guard.arm(owner, stop);
    CloseHandle(owner); CloseHandle(stop);
    if (FAILED(armed)) return 4;
    const auto probe = probe_virtual_camera(); // only registry and System32 exports
    if (guard.reason() != OwnerStopReason::none) return 4;
    std::cout << "{\"protocol\":1,\"api_exports_present\":" << (probe.api_exports_present ? "true" : "false")
        << ",\"machine_registration_present\":" << (probe.machine_registration_present ? "true" : "false")
        << ",\"user_registration_conflict\":" << (probe.user_registration_conflict ? "true" : "false")
        << ",\"api_error\":" << static_cast<unsigned long>(probe.api_error)
        << ",\"registration_error\":" << static_cast<unsigned long>(probe.registration_error)
        << ",\"runtime\":\"not_pinned\",\"media_started\":false}" << std::endl;
    guard.close();
    return FAILED(session.stop()) ? 5 : 0;
}
