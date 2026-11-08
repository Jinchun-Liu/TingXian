#include "capture_input.h"
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <wrl/client.h>
#include <new>

namespace tingxian::capture {
using Microsoft::WRL::ComPtr;
namespace {
struct ComScope {
    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    ~ComScope() { if (SUCCEEDED(hr)) CoUninitialize(); }
};
class WindowsInput final : public Backend {
    const DWORD owner_ = GetCurrentThreadId();
    std::unique_ptr<ComScope> com_;
    ComPtr<IAudioClient> client_;
    ComPtr<IAudioCaptureClient> capture_;
    HANDLE ready_ = nullptr;
    bool initialized_ = false, attempted_ = false;
    HRESULT prepare_inner(const Selection& selected, Format& out) {
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        if (attempted_) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
        auto hr = inspect(selected);
        if (hr != S_OK) return hr; // Re-query OS identity before IAudioClient activation.
        attempted_ = true;
        com_ = std::make_unique<ComScope>();
        if (FAILED(com_->hr)) return com_->hr;
        ComPtr<IMMDeviceEnumerator> enumerator;
        hr = CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&enumerator));
        if (FAILED(hr)) return hr;
        ComPtr<IMMDevice> device;
        hr = enumerator->GetDevice(selected.microphone_id.c_str(), &device);
        if (FAILED(hr)) return hr;
        DWORD state = 0;
        hr = device->GetState(&state);
        if (FAILED(hr)) return hr;
        if (state != DEVICE_STATE_ACTIVE) return HRESULT_FROM_WIN32(ERROR_DEVICE_NOT_CONNECTED);
        ComPtr<IMMEndpoint> endpoint;
        hr = device.As(&endpoint);
        if (FAILED(hr)) return hr;
        EDataFlow flow = eAll;
        hr = endpoint->GetDataFlow(&flow);
        if (FAILED(hr)) return hr;
        if (flow != eCapture) return E_ACCESSDENIED;
        hr = device->Activate(__uuidof(IAudioClient), CLSCTX_INPROC_SERVER, nullptr, &client_);
        if (FAILED(hr)) return hr;
        WAVEFORMATEX wave{};
        wave.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
        wave.nChannels = 1; wave.nSamplesPerSec = rate; wave.wBitsPerSample = 32;
        wave.nBlockAlign = static_cast<WORD>(sizeof(float)); wave.nAvgBytesPerSec = rate * UINT32(sizeof(float));
        // Client stream conversion only. No loopback, exclusive mode, default
        // device, device-format changes, global volume or recording-side effects.
        hr = client_->Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK |
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            200000, 0, &wave, nullptr);
        if (FAILED(hr)) return hr;
        initialized_ = true;
        UINT32 frames = 0;
        hr = client_->GetBufferSize(&frames);
        if (FAILED(hr)) return hr;
        if (!frames || frames > max_frames) return AUDCLNT_E_BUFFER_SIZE_ERROR;
        ready_ = CreateEventW(nullptr, FALSE, FALSE, nullptr);
        if (!ready_) return HRESULT_FROM_WIN32(GetLastError());
        hr = client_->SetEventHandle(ready_);
        if (FAILED(hr)) return hr;
        hr = client_->GetService(IID_PPV_ARGS(&capture_));
        if (FAILED(hr)) return hr;
        out = {rate, 1, frames};
        return S_OK;
    }
public:
    ~WindowsInput() override {
        stop_reset();
        capture_.Reset(); client_.Reset();
        if (ready_) CloseHandle(ready_);
        // com_ uninitializes only after every COM instance has been released.
    }
    HRESULT inspect(const Selection& selected) noexcept override {
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        std::vector<audio::Endpoint> endpoints;
        const auto hr = audio::enumerate_endpoints(endpoints);
        return hr == S_OK ? validate_selection(endpoints, selected) : hr;
    }
    HRESULT prepare(const Selection& selected, Format& out) noexcept override {
        out = {};
        try { return prepare_inner(selected, out); }
        catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
        catch (...) { return E_FAIL; }
    }
    HRESULT start() noexcept override {
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        return initialized_ && capture_ ? client_->Start() : E_UNEXPECTED;
    }
    HRESULT acquire(RawPacket& out) noexcept override {
        out = {};
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        if (!capture_) return E_UNEXPECTED;
        BYTE* data = nullptr;
        DWORD flags = 0;
        const auto hr = capture_->GetBuffer(&data, &out.frames, &flags, &out.first_frame, &out.qpc_100ns);
        out.flags = flags;
        if (hr == AUDCLNT_S_BUFFER_EMPTY) { out = {}; return S_FALSE; }
        if (hr == S_OK) { out.samples = reinterpret_cast<const float*>(data); return S_OK; }
        return FAILED(hr) ? hr : E_UNEXPECTED;
    }
    HRESULT release(uint32_t frames) noexcept override {
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        return capture_ ? capture_->ReleaseBuffer(frames) : E_UNEXPECTED;
    }
    HRESULT now(uint64_t& out) noexcept override {
        out = 0;
        LARGE_INTEGER ticks{}, frequency{};
        if (!QueryPerformanceCounter(&ticks) || !QueryPerformanceFrequency(&frequency) ||
            ticks.QuadPart < 0 || frequency.QuadPart <= 0 || frequency.QuadPart > 1000000000LL) return E_FAIL;
        const auto t = uint64_t(ticks.QuadPart), f = uint64_t(frequency.QuadPart);
        if (t / f > ((std::numeric_limits<uint64_t>::max)() - 10000000) / 10000000) return E_FAIL;
        out = (t / f) * 10000000 + ((t % f) * 10000000) / f;
        return S_OK;
    }
    Cleanup stop_reset() noexcept override {
        if (GetCurrentThreadId() != owner_) return {RPC_E_WRONG_THREAD, RPC_E_WRONG_THREAD, false};
        if (!initialized_) return {S_OK, S_OK, true};
        Cleanup out;
        out.stop_error = client_->Stop();
        out.reset_error = client_->Reset();
        out.reset = cleanup_success(out.stop_error) && cleanup_success(out.reset_error);
        return out;
    }
    HANDLE ready_event() const noexcept override { return ready_; }
};
}
std::unique_ptr<Backend> make_windows_backend() { return std::make_unique<WindowsInput>(); }
}
