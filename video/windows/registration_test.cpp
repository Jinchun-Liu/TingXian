// Explicit COM registration check only. It does not create a virtual device or
// enumerate/open physical cameras. Kept outside ordinary CTest until installed.
#include "video_contract.h"
#include "video_timing.h"
#include <mfapi.h>
#include <mfidl.h>
#include <wrl/client.h>
#include <iostream>
using Microsoft::WRL::ComPtr;
using namespace tingxian::video;
int wmain(int argc, wchar_t** argv) {
    if (argc != 2 || wcscmp(argv[1], L"--registered-no-device") != 0) return 2;
    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(hr)) return 1;
    const HRESULT startup = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    if (FAILED(startup)) { CoUninitialize(); return 1; }
    {
        ComPtr<IMFActivate> activation;
        hr = CoCreateInstance(SourceClass, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&activation));
        ComPtr<IMFMediaSource> source;
        if (SUCCEEDED(hr)) hr = activation->ActivateObject(IID_PPV_ARGS(&source));
        ComPtr<IFrameReceiver> frames;
        if (SUCCEEDED(hr)) hr = source.As(&frames);
        // FrameServer may release its activation while retaining the source.
        // The installed DLL must leave shutdown authority with that consumer.
        activation.Reset();
        FrameMetrics metrics{};
        if (SUCCEEDED(hr)) hr = frames->GetFrameMetrics(&metrics);
        if (SUCCEEDED(hr) && (metrics.enabled || metrics.accepted)) hr = E_UNEXPECTED;
        ComPtr<ITimedFrameReceiver> timing;
        if (SUCCEEDED(hr)) hr = source.As(&timing);
        UINT32 version = 0;
        if (SUCCEEDED(hr)) hr = timing->GetTimingVersion(&version);
        if (SUCCEEDED(hr) && version != 2) hr = E_UNEXPECTED;
        if (source) {
            const HRESULT stopped = source->Shutdown();
            if (SUCCEEDED(hr)) hr = stopped;
        }
        if (activation) {
            const HRESULT stopped = activation->ShutdownObject();
            if (SUCCEEDED(hr)) hr = stopped;
        }
    }
    MFShutdown(); CoUninitialize();
    if (FAILED(hr)) { std::cerr << "registered COM activation failed: 0x" << std::hex << unsigned(hr) << '\n'; return 1; }
    std::cout << "{\"registered_com_activation\":true,\"timing_version\":2,\"source_survives_activation_release\":true,\"default_output_enabled\":false,\"device_created\":false,\"physical_camera_opened\":false}\n";
    return 0;
}
