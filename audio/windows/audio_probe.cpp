#include "audio_output.h"
#include <iostream>
#include <iomanip>
int wmain(int argc, wchar_t** argv) {
    if (argc != 2 || (std::wstring(argv[1]) != L"--probe" && std::wstring(argv[1]) != L"--reject-unknown")) {
        std::cerr << "Usage: tingxian-audio-probe --probe | --reject-unknown (metadata only; no streams)\n";
        return 2;
    }
    if (std::wstring(argv[1]) == L"--reject-unknown") {
        auto backend = tingxian::audio::make_windows_backend();
        tingxian::audio::Format format;
        const auto hr = backend->prepare({L"tingxian-synthetic-nonexistent-render", L"tingxian-synthetic-nonexistent-capture"}, 2, format);
        if (hr != E_ACCESSDENIED || format.rate || backend->ready_event()) return 1;
        std::cout << "unknown_pair_rejected_before_audio_activation=true\n";
        return 0;
    }
    std::vector<tingxian::audio::Endpoint> endpoints;
    const HRESULT hr = tingxian::audio::enumerate_endpoints(endpoints);
    if (FAILED(hr)) {
        std::cerr << "enumeration_error=0x" << std::hex << static_cast<unsigned long>(hr) << '\n';
        return 1;
    }
    size_t active_render = 0, active_capture = 0, pairs = 0, topology_resolved = 0;
    for (const auto& e : endpoints) if (e.active) {
        if (e.topology_resolved) ++topology_resolved;
        if (e.flow == tingxian::audio::Flow::render) ++active_render; else ++active_capture;
        if (e.flow != tingxian::audio::Flow::render) continue;
        for (const auto& c : endpoints)
            if (tingxian::audio::validate_pair(endpoints, {e.id, c.id}) == S_OK) ++pairs;
    }
    // Counts only: private endpoint IDs/names stay in memory, not public logs.
    std::cout << "{\"metadata_only\":true,\"audio_clients_activated\":0,\"streams_started\":0,"
              << "\"endpoints\":" << endpoints.size() << ",\"active_render\":" << active_render
              << ",\"active_capture\":" << active_capture << ",\"candidate_pairs\":" << pairs
              << ",\"active_topologies_resolved\":" << topology_resolved
              << ",\"driver_signature_verified\":false,\"external_drain_verified\":false}\n";
    return 0; // Absence of optional cable is a supported inventory result, not an output pass.
}
