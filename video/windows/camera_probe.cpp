#include "camera_session.h"
#include <cstdio>
using namespace tingxian::video;
int wmain(int argc, wchar_t** argv) {
    if (argc != 2 || wcscmp(argv[1], L"--probe") != 0) {
        fputs("Usage: tingxian-video-device-probe --probe\nRead-only exports/registry inspection; no device is created.\n", stderr);
        return 2;
    }
    auto p = probe_virtual_camera();
    printf("{\"schema_version\":1,\"api_exports_present\":%s,\"machine_registration_present\":%s,"
        "\"user_registration_conflict\":%s,\"api_error\":\"0x%08lx\",\"registration_error\":\"0x%08lx\","
        "\"source_integrity_verified\":%s,\"device_created\":false,\"physical_camera_opened\":false,\"runtime_verified\":false}\n",
        p.api_exports_present ? "true" : "false", p.machine_registration_present ? "true" : "false",
        p.user_registration_conflict ? "true" : "false", static_cast<unsigned long>(p.api_error),
        static_cast<unsigned long>(p.registration_error), p.source_integrity_verified ? "true" : "false");
    return 0; // inspection completed, not an assertion that the component is ready
}
