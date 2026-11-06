// Real COM/MF integration with synthetic bytes only. No device registration,
// physical-camera enumeration, microphone, GPU inference or network activity.
#include "video_contract.h"
#include <mfapi.h>
#include <mfidl.h>
#include <mferror.h>
#include <wrl.h>
#include <atomic>
#include <iostream>
#include <stdexcept>
#include <thread>
#include <vector>

using Microsoft::WRL::ComPtr;
using namespace tingxian::video;
namespace {
int groups = 0;
void expect(bool ok, const char* description) { if (!ok) throw std::runtime_error(description); }
void check(HRESULT hr, const char* operation) {
    if (FAILED(hr)) { std::cerr << operation << " HRESULT=0x" << std::hex << unsigned(hr) << std::dec << '\n'; throw std::runtime_error(operation); }
}
void passed(const char* name) { ++groups; std::cout << "PASS " << name << '\n'; }
ComPtr<IMFMediaEvent> next_event(IMFMediaEventGenerator* generator, MediaEventType expected) {
    const auto deadline = GetTickCount64() + 2000;
    while (GetTickCount64() < deadline) {
        ComPtr<IMFMediaEvent> event;
        HRESULT hr = generator->GetEvent(MF_EVENT_FLAG_NO_WAIT, &event);
        if (hr == MF_E_NO_EVENTS_AVAILABLE) { Sleep(1); continue; }
        check(hr, "GetEvent");
        MediaEventType type; check(event->GetType(&type), "GetType");
        HRESULT status; check(event->GetStatus(&status), "GetStatus"); check(status, "event status");
        if (type == expected) return event;
    }
    throw std::runtime_error("expected event timed out");
}
ComPtr<IMFSample> next_sample(IMFMediaStream* stream, IUnknown* token = nullptr) {
    check(stream->RequestSample(token), "RequestSample");
    auto event = next_event(stream, MEMediaSample);
    PROPVARIANT value{};
    check(event->GetValue(&value), "sample event value");
    ComPtr<IMFSample> sample;
    HRESULT hr = value.vt == VT_UNKNOWN ? value.punkVal->QueryInterface(IID_PPV_ARGS(&sample)) : E_UNEXPECTED;
    PropVariantClear(&value); check(hr, "sample interface");
    return sample;
}
void pixels(IMFSample* sample, BYTE y_value, BYTE uv_value) {
    ComPtr<IMFMediaBuffer> buffer; check(sample->GetBufferByIndex(0, &buffer), "sample buffer");
    ComPtr<IMF2DBuffer2> image; check(buffer.As(&image), "2D buffer");
    BYTE *scanline = nullptr, *allocation = nullptr; LONG pitch = 0; DWORD length = 0;
    check(image->Lock2DSize(MF2DBuffer_LockFlags_Read, &scanline, &pitch, &allocation, &length), "Lock2DSize");
    bool valid = pitch >= LONG(Width);
    if (valid) for (UINT32 row = 0; row < Height + Height / 2; ++row) {
        const BYTE expected = row < Height ? y_value : uv_value;
        for (UINT32 col = 0; col < Width; ++col) if (scanline[size_t(row) * pitch + col] != expected) valid = false;
    }
    check(image->Unlock2D(), "Unlock2D");
    expect(valid, "visible NV12 bytes differ");
}
std::vector<BYTE> frame(BYTE y, BYTE uv) {
    std::vector<BYTE> bytes(FrameBytes, uv);
    std::fill_n(bytes.begin(), Width * Height, y);
    return bytes;
}
void run(HMODULE library) {
    const auto get_class = reinterpret_cast<HRESULT(__stdcall*)(REFCLSID, REFIID, void**)>(GetProcAddress(library, "DllGetClassObject"));
    const auto unload = reinterpret_cast<HRESULT(__stdcall*)()>(GetProcAddress(library, "DllCanUnloadNow"));
    expect(get_class && unload, "DLL exports missing");
    expect(unload() == S_OK, "empty DLL not unloadable");
    expect(get_class(SourceClass, IID_IClassFactory, nullptr) == E_POINTER, "null factory output not rejected");
    ComPtr<IClassFactory> factory;
    expect(get_class(GUID_NULL, IID_PPV_ARGS(&factory)) == CLASS_E_CLASSNOTAVAILABLE && !factory, "foreign CLSID accepted");
    check(get_class(SourceClass, IID_PPV_ARGS(&factory)), "class factory");
    expect(unload() == S_FALSE, "live factory not tracked");
    passed("class identity and DLL lifetime");
    {
        ComPtr<IMFActivate> activation;
        check(factory->CreateInstance(nullptr, IID_PPV_ARGS(&activation)), "activation");
        ComPtr<IMFMediaSourceEx> source;
        check(activation->ActivateObject(IID_PPV_ARGS(&source)), "ActivateObject");
        ComPtr<IMFMediaSource> base; check(source.As(&base), "base source QI");
        ComPtr<IMFMediaEventGenerator> generator; check(source.As(&generator), "event generator QI");
        ComPtr<IFrameReceiver> receiver; check(source.As(&receiver), "frame receiver QI");
        ComPtr<IMFSampleAllocatorControl> allocation; check(source.As(&allocation), "allocator control QI");
        DWORD input = 9; MFSampleAllocatorUsage usage{};
        check(allocation->GetAllocatorUsage(0, &input, &usage), "GetAllocatorUsage");
        expect(input == 0 && usage == MFSampleAllocatorUsage_UsesProvidedAllocator, "allocator contract differs");
        expect(allocation->SetDefaultAllocator(0, nullptr) == E_POINTER, "null allocator not rejected");
        expect(allocation->GetAllocatorUsage(1, &input, &usage) == MF_E_INVALIDSTREAMNUMBER, "foreign stream accepted");
        passed("interfaces and allocator contract");

        ComPtr<IMFPresentationDescriptor> pd; check(source->CreatePresentationDescriptor(&pd), "presentation");
        DWORD count = 0; check(pd->GetStreamDescriptorCount(&count), "stream count"); expect(count == 1, "more than one stream");
        BOOL selected = FALSE; ComPtr<IMFStreamDescriptor> sd;
        check(pd->GetStreamDescriptorByIndex(0, &selected, &sd), "stream descriptor"); expect(selected, "stream not selected");
        ComPtr<IMFMediaTypeHandler> handler; check(sd->GetMediaTypeHandler(&handler), "type handler");
        check(handler->GetMediaTypeCount(&count), "type count"); expect(count == 1, "unexpected format advertised");
        ComPtr<IMFMediaType> type; check(handler->GetCurrentMediaType(&type), "current type");
        UINT32 width = 0, height = 0, numerator = 0, denominator = 0; GUID subtype{};
        check(MFGetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, &width, &height), "size");
        check(MFGetAttributeRatio(type.Get(), MF_MT_FRAME_RATE, &numerator, &denominator), "rate");
        check(type->GetGUID(MF_MT_SUBTYPE, &subtype), "subtype");
        expect(width == 1280 && height == 720 && numerator == 30 && denominator == 1 && subtype == MFVideoFormat_NV12, "not 720p30 NV12");
        passed("single 720p30 NV12 stream with no audio");

        PROPVARIANT position{};
        expect(source->Start(pd.Get(), nullptr, &position) == MF_E_NOT_INITIALIZED, "start without allocator succeeded");
        ComPtr<IMFVideoSampleAllocator> allocator;
        check(MFCreateVideoSampleAllocatorEx(IID_PPV_ARGS(&allocator)), "create sample allocator");
        check(allocation->SetDefaultAllocator(0, allocator.Get()), "set sample allocator");
        check(source->Start(pd.Get(), nullptr, &position), "source start");
        auto announced = next_event(source.Get(), MENewStream);
        PROPVARIANT stream_value{}; check(announced->GetValue(&stream_value), "new stream value");
        ComPtr<IMFMediaStream2> stream;
        HRESULT stream_hr = stream_value.vt == VT_UNKNOWN ? stream_value.punkVal->QueryInterface(IID_PPV_ARGS(&stream)) : E_UNEXPECTED;
        PropVariantClear(&stream_value); check(stream_hr, "stream interface"); announced.Reset();
        next_event(source.Get(), MESourceStarted); next_event(stream.Get(), MEStreamStarted);
        passed("failed prepare recovers and MF start events arrive");

        ComPtr<IMFAttributes> token; check(MFCreateAttributes(&token, 1), "token");
        auto neutral = next_sample(stream.Get(), token.Get()); pixels(neutral.Get(), 32, 128);
        ComPtr<IUnknown> returned_token; check(neutral->GetUnknown(MFSampleExtension_Token, IID_PPV_ARGS(&returned_token)), "request token");
        expect(returned_token.Get() == token.Get(), "request token lost");
        LONGLONG previous = 0, duration = 0; check(neutral->GetSampleTime(&previous), "sample timestamp");
        check(neutral->GetSampleDuration(&duration), "sample duration"); expect(duration == FrameDuration, "duration differs");
        neutral.Reset(); returned_token.Reset();
        passed("default neutral pixels and request-token preservation");

        auto first = frame(160, 96); auto second = frame(90, 140);
        expect(FAILED(receiver->SubmitNv12(0, 1, GetTickCount64(), first.data(), FrameBytes)), "unstarted submit accepted");
        UINT64 generation = 0; check(receiver->BeginVideo(&generation), "begin video");
        check(receiver->SubmitNv12(generation, 1, GetTickCount64(), first.data(), FrameBytes), "submit first frame");
        auto sample = next_sample(stream.Get()); pixels(sample.Get(), 160, 96);
        LONGLONG current = 0; check(sample->GetSampleTime(&current), "next timestamp"); expect(current > previous, "timestamp did not increase");
        sample.Reset();
        auto duplicate = next_sample(stream.Get()); pixels(duplicate.Get(), 160, 96); duplicate.Reset();
        FrameMetrics metrics{}; check(receiver->GetFrameMetrics(&metrics), "frame metrics");
        expect(metrics.unique_samples == 1 && metrics.repeated_samples == 1 && metrics.neutral_samples == 1, "repeat counted as unique");
        passed("actual sample bytes and unique/repeated/neutral counters");

        expect(FAILED(receiver->SubmitNv12(generation, 1, GetTickCount64(), first.data(), FrameBytes)), "duplicate id accepted");
        expect(FAILED(receiver->SubmitNv12(generation + 1, 2, GetTickCount64(), first.data(), FrameBytes)), "wrong generation accepted");
        expect(FAILED(receiver->SubmitNv12(generation, 2, GetTickCount64() + 10000, first.data(), FrameBytes)), "future timestamp accepted");
        expect(FAILED(receiver->SubmitNv12(generation, 2, GetTickCount64(), first.data(), FrameBytes - 1)), "short frame accepted");
        passed("stale identifiers and malformed frames rejected");

        check(receiver->SubmitNv12(generation, 2, GetTickCount64(), first.data(), FrameBytes), "queued first");
        check(receiver->SubmitNv12(generation, 3, GetTickCount64(), second.data(), FrameBytes), "replace pending");
        auto latest = next_sample(stream.Get()); pixels(latest.Get(), 90, 140); latest.Reset();
        check(receiver->GetFrameMetrics(&metrics), "drop metrics"); expect(metrics.dropped == 1, "replaced input not counted");
        passed("one-frame pending storage drops obsolete input");

        check(receiver->StopVideo(), "stop video");
        expect(FAILED(receiver->SubmitNv12(generation, 4, GetTickCount64(), first.data(), FrameBytes)), "late submit accepted");
        auto stopped = next_sample(stream.Get()); pixels(stopped.Get(), 32, 128); stopped.Reset();
        UINT64 newer = 0; check(receiver->BeginVideo(&newer), "restart video"); expect(newer > generation, "generation reused");
        check(receiver->SubmitNv12(newer, 1, GetTickCount64(), first.data(), FrameBytes), "restart submit");
        auto resumed = next_sample(stream.Get()); pixels(resumed.Get(), 160, 96); resumed.Reset();
        passed("stop neutralizes output and restart needs a new generation");

        Sleep(DWORD(MaxFrameAgeMs + 100));
        auto expired = next_sample(stream.Get()); pixels(expired.Get(), 32, 128); expired.Reset();
        check(receiver->GetFrameMetrics(&metrics), "watchdog metrics"); expect(!metrics.enabled, "watchdog did not stop generation");
        expect(FAILED(receiver->SubmitNv12(newer, 2, GetTickCount64(), first.data(), FrameBytes)), "expired generation resurrected");
        passed("output-side lease expires without a worker callback");

        check(receiver->BeginVideo(&newer), "begin race generation");
        std::atomic<bool> start{false};
        std::thread producer([&] {
            while (!start.load()) std::this_thread::yield();
            for (UINT64 id = 1; id <= 64; ++id) receiver->SubmitNv12(newer, id, GetTickCount64(), first.data(), FrameBytes);
        });
        start = true;
        check(receiver->StopVideo(), "stop concurrent producer");
        producer.join();
        auto after_race = next_sample(stream.Get()); pixels(after_race.Get(), 32, 128); after_race.Reset();
        expect(FAILED(receiver->SubmitNv12(newer, 65, GetTickCount64(), first.data(), FrameBytes)), "concurrent stop resurrected stream");
        passed("concurrent old producer cannot refill stopped output");

        check(source->Stop(), "MF source stop");
        next_event(source.Get(), MESourceStopped); next_event(stream.Get(), MEStreamStopped);
        expect(FAILED(stream->RequestSample(nullptr)), "stopped stream produced a sample");
        check(source->Start(pd.Get(), nullptr, &position), "MF source restart");
        next_event(source.Get(), MEUpdatedStream); next_event(source.Get(), MESourceStarted); next_event(stream.Get(), MEStreamStarted);
        auto recovered = next_sample(stream.Get()); pixels(recovered.Get(), 32, 128); recovered.Reset();
        passed("MF restart preserves neutral default");

        std::vector<ComPtr<IMFSample>> held;
        for (int i = 0; i < 4; ++i) held.push_back(next_sample(stream.Get()));
        expect(FAILED(stream->RequestSample(nullptr)), "output allocator grew past its bound");
        held.clear();
        auto released = next_sample(stream.Get()); pixels(released.Get(), 32, 128); released.Reset();
        passed("bounded output pool recovers after consumer releases samples");

        check(source->Shutdown(), "source shutdown"); check(source->Shutdown(), "repeat shutdown");
        expect(receiver->BeginVideo(&newer) == MF_E_SHUTDOWN, "shutdown source accepted begin");
        expect(stream->RequestSample(nullptr) == MF_E_SHUTDOWN, "shutdown stream accepted request");
        check(activation->ShutdownObject(), "activation shutdown");
        passed("shutdown is idempotent and rejects further work");
    }
    {
        ComPtr<IMFActivate> activation;
        check(factory->CreateInstance(nullptr, IID_PPV_ARGS(&activation)), "lifetime activation");
        ComPtr<IMFMediaSource> source;
        check(activation->ActivateObject(IID_PPV_ARGS(&source)), "lifetime source");
        activation.Reset();
        DWORD flags = 0;
        const HRESULT live = source->GetCharacteristics(&flags);
        const HRESULT shutdown = source->Shutdown(); // clean up even on a regression
        check(live, "source must survive activation release");
        expect(flags == MFMEDIASOURCE_IS_LIVE, "source lost live characteristic");
        check(shutdown, "consumer-owned source shutdown");
        expect(source->GetCharacteristics(&flags) == MF_E_SHUTDOWN, "explicit shutdown ignored");
        passed("source remains live until its consumer explicitly shuts it down");
    }
    check(factory->LockServer(TRUE), "server lock"); factory.Reset();
    expect(unload() == S_FALSE, "server lock ignored");
    check(get_class(SourceClass, IID_PPV_ARGS(&factory)), "factory for unlock");
    check(factory->LockServer(FALSE), "server unlock"); factory.Reset();
    expect(unload() == S_OK, "COM or sample references leaked");
    passed("all component references released without registration");
}
}

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) { std::cerr << "Usage: tingxian-video-source-test <absolute DLL path>\n"; return 2; }
    HMODULE library = nullptr;
    bool com = false, mf = false;
    int result = 1;
    try {
        check(CoInitializeEx(nullptr, COINIT_MULTITHREADED), "CoInitializeEx"); com = true;
        check(MFStartup(MF_VERSION, MFSTARTUP_FULL), "MFStartup"); mf = true;
        library = LoadLibraryExW(argv[1], nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
        if (!library) check(HRESULT_FROM_WIN32(GetLastError()), "load source DLL");
        run(library);
        std::cout << "{\"checks\":" << groups << ",\"result\":\"pass\",\"synthetic_only\":true,\"device_registered\":false,\"physical_camera_opened\":false}\n";
        result = 0;
    } catch (const std::exception& error) { std::cerr << "FAIL " << error.what() << '\n'; }
    if (library) FreeLibrary(library);
    if (mf) MFShutdown();
    if (com) CoUninitialize();
    return result;
}
