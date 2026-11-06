// Explicit installed-device test. The only input is generated gray NV12 pixels.
// Uses the production session backend's own shared preview; never enumerates and
// selects a physical camera, never opens audio, and never records images.
#include "camera_session.h"
#include <mfapi.h>
#include <mferror.h>
#include <mfreadwrite.h>
#include <appmodel.h>
#include <wrl.h>
#include <chrono>
#include <condition_variable>
#include <cstring>
#include <iostream>
#include <memory>
#include <set>
#include <string>
#include <vector>

using namespace tingxian::video;
using namespace Microsoft::WRL;
namespace {
void require(HRESULT hr, const char* stage) {
    std::cout << "{\"stage\":\"" << stage << "\",\"hr\":\"0x" << std::hex
        << static_cast<unsigned long>(hr) << std::dec << "\"}" << std::endl;
    if (FAILED(hr)) throw hr;
}
struct ComApartment {
    HRESULT result = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    ~ComApartment() { if (SUCCEEDED(result)) CoUninitialize(); }
};
struct MediaFoundation {
    HRESULT result = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    ~MediaFoundation() { if (SUCCEEDED(result)) MFShutdown(); }
};
struct CoString {
    wchar_t* value = nullptr;
    ~CoString() { CoTaskMemFree(value); }
};
class ReaderCallback final : public RuntimeClass<RuntimeClassFlags<ClassicCom>, IMFSourceReaderCallback> {
    std::mutex lock_;
    std::condition_variable ready_;
    bool received_ = false;
    HRESULT result_ = S_OK;
    ComPtr<IMFSample> sample_;
public:
    STDMETHODIMP OnReadSample(HRESULT hr, DWORD, DWORD flags, LONGLONG, IMFSample* sample) override {
        std::lock_guard guard(lock_);
        result_ = (flags & MF_SOURCE_READERF_ERROR) && SUCCEEDED(hr) ? E_FAIL : hr;
        sample_ = sample; received_ = true; ready_.notify_all(); return S_OK;
    }
    STDMETHODIMP OnFlush(DWORD) override { return S_OK; }
    STDMETHODIMP OnEvent(DWORD, IMFMediaEvent*) override { return S_OK; }
    HRESULT read(IMFSourceReader* reader, IMFSample** sample) {
        if (!sample) return E_POINTER;
        *sample = nullptr;
        {
            std::lock_guard guard(lock_);
            sample_.Reset(); received_ = false; result_ = S_OK;
        }
        HRESULT hr = reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0, nullptr, nullptr, nullptr, nullptr);
        if (FAILED(hr)) return hr;
        std::unique_lock guard(lock_);
        if (!ready_.wait_for(guard, std::chrono::seconds(5), [&] { return received_; }))
            return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        if (FAILED(result_)) return result_;
        return sample_.CopyTo(sample);
    }
};
struct SyntheticProducer final : OutputLease {
    FramePublisher publisher;
    std::atomic<bool> revoked{false};
    std::atomic<HRESULT> error{S_OK};
    std::atomic<unsigned> stage{0}; // 0 waiting, 1 accepted, 2 begun, 3 submitting
    std::atomic<UINT64> submitted{0};
    std::thread sender;
    ~SyntheticProducer() { revoke(); if (sender.joinable()) sender.join(); }
    void revoke() noexcept override { revoked = true; publisher.close(); }
    void start() {
        sender = std::thread([this] {
            HRESULT hr = publisher.accept();
            if (SUCCEEDED(hr)) { stage = 1; hr = publisher.begin(1); }
            if (SUCCEEDED(hr)) stage = 2;
            std::vector<BYTE> pixels(FrameBytes, 128);
            UINT64 id = 0;
            while (SUCCEEDED(hr) && !revoked.load()) {
                ++id;
                stage = 3;
                std::fill_n(pixels.begin(), Width * Height, BYTE(64 + id % 128));
                hr = publisher.submit(1, id, GetTickCount64(), pixels.data(), FrameBytes);
                if (SUCCEEDED(hr)) submitted = id;
                std::this_thread::sleep_for(std::chrono::milliseconds(33));
            }
            if (!revoked.load()) error = hr;
        });
    }
};
// Inspect all active pixels, accounting for 2D stride. Only numeric classification
// is emitted. No buffer/image or camera identifier is logged or written to disk.
HRESULT classify(IMFSample* sample, BYTE* luma) {
    if (!sample || !luma) return E_POINTER;
    ComPtr<IMFMediaBuffer> buffer;
    HRESULT hr = sample->ConvertToContiguousBuffer(&buffer); if (FAILED(hr)) return hr;
    std::vector<BYTE> pixels(FrameBytes);
    ComPtr<IMF2DBuffer> planar;
    if (SUCCEEDED(buffer.As(&planar))) {
        hr = planar->ContiguousCopyTo(pixels.data(), FrameBytes); if (FAILED(hr)) return hr;
    } else {
        BYTE* data = nullptr; DWORD capacity = 0, length = 0;
        hr = buffer->Lock(&data, &capacity, &length); if (FAILED(hr)) return hr;
        if (length == FrameBytes) std::memcpy(pixels.data(), data, FrameBytes);
        const auto unlocked = buffer->Unlock();
        if (FAILED(unlocked)) return unlocked;
        if (length != FrameBytes) return MF_E_BUFFERTOOSMALL;
    }
    *luma = pixels[0];
    if (*luma != 32 && (*luma < 64 || *luma > 191)) return E_UNEXPECTED;
    for (UINT32 i = 0; i < FrameBytes; ++i)
        if (pixels[i] != (i < Width * Height ? *luma : 128)) return E_UNEXPECTED;
    return S_OK;
}
// Enumeration is metadata only, used solely to match this newly created device's
// exact symbolic link. Never log other devices or activate an enumerated source.
HRESULT present(const wchar_t* expected, bool* found) {
    if (!expected || !found) return E_POINTER;
    *found = false;
    ComPtr<IMFAttributes> attributes;
    HRESULT hr = MFCreateAttributes(&attributes, 1); if (FAILED(hr)) return hr;
    hr = attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
    if (FAILED(hr)) return hr;
    IMFActivate** devices = nullptr; UINT32 count = 0;
    hr = MFEnumDeviceSources(attributes.Get(), &devices, &count); if (FAILED(hr)) return hr;
    for (UINT32 i = 0; i < count; ++i) {
        CoString value; UINT32 length = 0;
        const auto read = devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
            &value.value, &length);
        if (SUCCEEDED(read) && value.value && _wcsicmp(value.value, expected) == 0) *found = true;
        devices[i]->Release();
    }
    CoTaskMemFree(devices); return S_OK;
}
HRESULT run_cycle(unsigned cycle) {
    SyntheticProducer producer;
    std::unique_ptr<CameraBackend> backend(make_windows_camera_backend());
    if (!backend) return E_OUTOFMEMORY;
    CameraSession session(*backend, producer);
    ComPtr<IMFMediaSource> preview;
    ComPtr<IMFSourceReader> reader;
    auto callback = Make<ReaderCallback>();
    CoString link;
    HRESULT original = S_OK;
    try {
        require(callback ? S_OK : E_OUTOFMEMORY, "callback");
        VideoPipeEndpoint endpoint;
        require(producer.publisher.open(&endpoint), "producer_open");
        producer.start();
        const HRESULT started = session.start(endpoint);
        std::cout << "{\"backend_startup_stage\":\"" << backend->startup_stage() << "\"}" << std::endl;
        require(started, "session_start");
        UINT32 length = 0;
        require(backend->symbolic_link(&link.value, &length), "own_symbolic_link");
        require(link.value && length ? S_OK : E_UNEXPECTED, "nonempty_own_link");
        bool listed = false;
        require(present(link.value, &listed), "enumerate_metadata");
        require(listed ? S_OK : E_UNEXPECTED, "own_device_present");
        require(backend->preview_source(&preview), "own_shared_preview");
        ComPtr<IMFAttributes> options;
        require(MFCreateAttributes(&options, 1), "reader_options");
        require(options->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, callback.Get()), "async_callback");
        require(MFCreateSourceReaderFromMediaSource(preview.Get(), options.Get(), &reader), "source_reader");
        ComPtr<IMFMediaType> type;
        require(MFCreateMediaType(&type), "nv12_type");
        require(type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "video_only");
        require(type->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12), "nv12");
        require(MFSetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, Width, Height), "720p");
        require(MFSetAttributeRatio(type.Get(), MF_MT_FRAME_RATE, 30, 1), "30fps_format");
        require(reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM, nullptr, type.Get()), "reader_format");
        std::set<BYTE> unique;
        UINT64 neutral_samples = 0, empty_samples = 0;
        const auto deadline = GetTickCount64() + 5000;
        while (unique.size() < 3 && GetTickCount64() < deadline) {
            ComPtr<IMFSample> sample;
            HRESULT hr = callback->read(reader.Get(), &sample); if (FAILED(hr)) require(hr, "read_live");
            if (sample) {
                BYTE luma = 0; hr = classify(sample.Get(), &luma); if (FAILED(hr)) require(hr, "validate_pixels");
                if (luma != 32) unique.insert(luma); else ++neutral_samples;
            } else ++empty_samples;
            std::this_thread::sleep_for(std::chrono::milliseconds(5));
        }
        std::cout << "{\"producer_stage\":" << producer.stage.load()
            << ",\"producer_error\":\"0x" << std::hex << static_cast<unsigned long>(producer.error.load()) << std::dec
            << "\",\"submitted\":" << producer.submitted.load() << ",\"distinct_live\":" << unique.size()
            << ",\"neutral_samples\":" << neutral_samples << ",\"empty_samples\":" << empty_samples << "}" << std::endl;
        require(unique.size() == 3 && SUCCEEDED(producer.error.load()) ? S_OK : E_UNEXPECTED, "three_distinct_synthetic_frames");
        producer.revoke();
        if (producer.sender.joinable()) producer.sender.join();
        bool neutral = false;
        const auto neutral_deadline = GetTickCount64() + 1000;
        while (!neutral && GetTickCount64() < neutral_deadline) {
            ComPtr<IMFSample> sample;
            HRESULT hr = callback->read(reader.Get(), &sample); if (FAILED(hr)) require(hr, "read_neutral");
            if (sample) {
                BYTE luma = 0; hr = classify(sample.Get(), &luma); if (FAILED(hr)) require(hr, "validate_neutral_pixels");
                neutral = luma == 32;
            }
        }
        require(neutral && GetTickCount64() <= neutral_deadline ? S_OK : HRESULT_FROM_WIN32(ERROR_TIMEOUT), "neutral_after_producer_stop");
        std::cout << "{\"cycle\":" << cycle << ",\"submitted\":" << producer.submitted.load()
            << ",\"distinct_synthetic_frames\":" << unique.size() << ",\"neutral_observed\":true}" << std::endl;
    } catch (HRESULT hr) { original = hr; }
    catch (...) { original = E_FAIL; }
    // All acquired handles have a cleanup path, including failed Start/preview.
    session.request_stop();
    reader.Reset(); // callback refcounts remain owned until the reader has released them
    if (preview) { const auto hr = preview->Shutdown(); if (FAILED(hr) && SUCCEEDED(original)) original = hr; }
    preview.Reset(); callback.Reset();
    const auto stopped = session.stop();
    std::cout << "{\"stage\":\"session_cleanup\",\"hr\":\"0x" << std::hex
        << static_cast<unsigned long>(stopped) << std::dec << "\"}" << std::endl;
    if (FAILED(stopped) && SUCCEEDED(original)) original = stopped;
    if (link.value && SUCCEEDED(stopped)) {
        bool listed = true; HRESULT hr = S_OK;
        const auto deadline = GetTickCount64() + 5000;
        do {
            hr = present(link.value, &listed);
            if (FAILED(hr) || !listed) break;
            std::this_thread::sleep_for(std::chrono::milliseconds(100));
        } while (GetTickCount64() < deadline);
        if (SUCCEEDED(hr) && listed) hr = HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        std::cout << "{\"stage\":\"device_absent_after_shutdown\",\"hr\":\"0x" << std::hex
            << static_cast<unsigned long>(hr) << std::dec << "\"}" << std::endl;
        if (FAILED(hr) && SUCCEEDED(original)) original = hr;
    }
    return original;
}
}
int wmain(int argc, wchar_t** argv) {
    if (argc == 2 && wcscmp(argv[1], L"--describe") == 0) {
        std::cout << "{\"mode\":\"description_only\",\"device_created\":false,\"physical_camera_opened\":false}\n";
        return 0;
    }
    if (argc != 2 || wcscmp(argv[1], L"--run-synthetic-device") != 0) {
        std::cerr << "Use --describe (no side effects), or --run-synthetic-device after component installation and explicit authorization.\n";
        return 2;
    }
    std::cout << "{\"mode\":\"actual_virtual_device_synthetic_pixels\",\"physical_camera_opened\":false,\"audio_opened\":false}" << std::endl;
    UINT32 package_length = 0;
    const LONG package_result = GetCurrentPackageFullName(&package_length, nullptr);
    std::cout << "{\"package_identity_present\":" << (package_result == ERROR_INSUFFICIENT_BUFFER ? "true" : "false")
        << ",\"package_query_status\":" << package_result << "}" << std::endl;
    ComApartment apartment;
    if (FAILED(apartment.result)) return 1;
    MediaFoundation mf;
    if (FAILED(mf.result)) return 1;
    for (unsigned cycle = 1; cycle <= 3; ++cycle) {
        const auto hr = run_cycle(cycle);
        if (FAILED(hr)) {
            std::cout << "{\"result\":\"fail\",\"cycle\":" << cycle << ",\"hr\":\"0x"
                << std::hex << static_cast<unsigned long>(hr) << std::dec << "\"}" << std::endl;
            return 1;
        }
    }
    std::cout << "{\"result\":\"pass\",\"cycles\":3,\"physical_camera_opened\":false,\"audio_opened\":false,\"eye_sdk_loaded\":false}\n";
    return 0;
}
