#include "audio_output.h"
#include <initguid.h>
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <functiondiscoverykeys_devpkey.h>
#include <devpkey.h>
#include <devicetopology.h>
#include <cfgmgr32.h>
#include <wrl/client.h>
#include <algorithm>
#include <cmath>
#include <limits>
#include <stdexcept>

namespace tingxian::audio {
using Microsoft::WRL::ComPtr;
namespace {
struct ComScope {
    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    ~ComScope() { if (SUCCEEDED(hr)) CoUninitialize(); }
};
struct TaskString {
    wchar_t* value = nullptr;
    ~TaskString() { CoTaskMemFree(value); }
};
struct Property {
    PROPVARIANT value;
    Property() { PropVariantInit(&value); }
    ~Property() { PropVariantClear(&value); }
};
std::wstring property(IPropertyStore* store, REFPROPERTYKEY key) {
    Property p;
    if (FAILED(store->GetValue(key, &p.value)) || p.value.vt != VT_LPWSTR || !p.value.pwszVal) return {};
    return p.value.pwszVal;
}
std::vector<std::wstring> node_strings(DEVINST node, const DEVPROPKEY& key, DEVPROPTYPE expected) {
    ULONG bytes = 0;
    DEVPROPTYPE type = 0;
    if (CM_Get_DevNode_PropertyW(node, &key, &type, nullptr, &bytes, 0) != CR_BUFFER_SMALL ||
        bytes < sizeof(wchar_t) || bytes > 65536 || bytes % sizeof(wchar_t)) return {};
    std::vector<wchar_t> data(bytes / sizeof(wchar_t));
    const auto capacity = bytes;
    if (CM_Get_DevNode_PropertyW(node, &key, &type, reinterpret_cast<BYTE*>(data.data()), &bytes, 0) != CR_SUCCESS ||
        type != expected || bytes > capacity || bytes < sizeof(wchar_t) || bytes % sizeof(wchar_t)) return {};
    data.resize(bytes / sizeof(wchar_t));
    if (data.back() != L'\0') return {};
    std::vector<std::wstring> result;
    size_t offset = 0;
    while (offset < data.size() && data[offset]) {
        const auto end = std::find(data.begin() + offset, data.end(), L'\0');
        if (end == data.end()) return {};
        result.emplace_back(data.data() + offset, static_cast<size_t>(end - data.begin()) - offset);
        offset = static_cast<size_t>(end - data.begin()) + 1;
        if (expected == DEVPROP_TYPE_STRING) break;
    }
    return result;
}
void find_adapter(const std::wstring& instance, Endpoint& endpoint) {
    if (instance.empty()) return;
    auto mutable_id = instance;
    DEVINST node = 0;
    if (CM_Locate_DevNodeW(&node, mutable_id.data(), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS) return;
    for (unsigned depth = 0; depth < 16; ++depth) {
        const auto hardware = node_strings(node, DEVPKEY_Device_HardwareIds, DEVPROP_TYPE_STRING_LIST);
        const auto service = node_strings(node, DEVPKEY_Device_Service, DEVPROP_TYPE_STRING);
        wchar_t id[MAX_DEVICE_ID_LEN] = {};
        if (service.size() == 1 && CM_Get_Device_IDW(node, id, MAX_DEVICE_ID_LEN, 0) == CR_SUCCESS) {
            for (const auto& h : hardware) {
                Endpoint candidate;
                candidate.adapter_id = id;
                candidate.hardware_id = h;
                candidate.service = service.front();
                if (is_basic_cable(candidate)) {
                    endpoint.adapter_id = candidate.adapter_id;
                    endpoint.hardware_id = h;
                    endpoint.service = candidate.service;
                    return;
                }
            }
        }
        DEVINST parent = 0;
        if (CM_Get_Parent(&parent, node, 0) != CR_SUCCESS || parent == node) return;
        node = parent;
    }
}
std::wstring connected_adapter(IMMDevice* endpoint, IMMDeviceEnumerator* enumerator) {
    // An endpoint's SWD node is not itself the audio adapter. Resolve the
    // topology connection first, then inspect that device's PnP properties.
    // This activates a metadata interface only; no audio stream or controls.
    ComPtr<IDeviceTopology> topology;
    if (FAILED(endpoint->Activate(__uuidof(IDeviceTopology), CLSCTX_INPROC_SERVER, nullptr, &topology))) return {};
    UINT count = 0;
    if (FAILED(topology->GetConnectorCount(&count)) || count != 1) return {};
    ComPtr<IConnector> connector;
    if (FAILED(topology->GetConnector(0, &connector))) return {};
    TaskString id;
    if (FAILED(connector->GetDeviceIdConnectedTo(&id.value))) return {};
    ComPtr<IMMDevice> adapter;
    if (FAILED(enumerator->GetDevice(id.value, &adapter))) return {};
    ComPtr<IPropertyStore> store;
    if (FAILED(adapter->OpenPropertyStore(STGM_READ, &store))) return {};
    return property(store.Get(), PKEY_Device_InstanceId);
}
HRESULT enumerate_on_mta(std::vector<Endpoint>& endpoints) {
    ComPtr<IMMDeviceEnumerator> enumerator;
    HRESULT hr = CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_INPROC_SERVER,
                                  IID_PPV_ARGS(&enumerator));
    if (FAILED(hr)) return hr;
    ComPtr<IMMDeviceCollection> collection;
    hr = enumerator->EnumAudioEndpoints(eAll, DEVICE_STATEMASK_ALL, &collection);
    if (FAILED(hr)) return hr;
    UINT count = 0;
    hr = collection->GetCount(&count);
    if (FAILED(hr)) return hr;
    if (count > 1024) return HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW);
    std::vector<Endpoint> result;
    for (UINT i = 0; i < count; ++i) {
        ComPtr<IMMDevice> device;
        hr = collection->Item(i, &device);
        if (FAILED(hr)) return hr;
        TaskString id;
        hr = device->GetId(&id.value);
        if (FAILED(hr)) return hr;
        DWORD state = 0;
        hr = device->GetState(&state);
        if (FAILED(hr)) return hr;
        ComPtr<IMMEndpoint> endpoint;
        hr = device.As(&endpoint);
        if (FAILED(hr)) return hr;
        EDataFlow flow = eAll;
        hr = endpoint->GetDataFlow(&flow);
        if (FAILED(hr)) return hr;
        if (flow != eRender && flow != eCapture) return E_UNEXPECTED;
        Endpoint e;
        e.id = id.value;
        e.active = state == DEVICE_STATE_ACTIVE;
        e.flow = flow == eRender ? Flow::render : Flow::capture;
        ComPtr<IPropertyStore> store;
        hr = device->OpenPropertyStore(STGM_READ, &store);
        if (FAILED(hr)) return hr;
        e.name = property(store.Get(), PKEY_Device_FriendlyName);
        Property form;
        if (SUCCEEDED(store->GetValue(PKEY_AudioEndpoint_FormFactor, &form.value)) && form.value.vt == VT_UI4)
            e.form_factor = form.value.ulVal;
        if (e.active) {
            const auto instance = connected_adapter(device.Get(), enumerator.Get());
            e.topology_resolved = !instance.empty();
            find_adapter(instance, e);
        }
        result.push_back(std::move(e));
    }
    endpoints = std::move(result);
    return S_OK;
}
HRESULT current_qpc(uint64_t& value) noexcept {
    LARGE_INTEGER ticks{}, frequency{};
    if (!QueryPerformanceCounter(&ticks) || !QueryPerformanceFrequency(&frequency) || ticks.QuadPart < 0 ||
        frequency.QuadPart <= 0 || frequency.QuadPart > 1000000000LL) return E_FAIL;
    const auto t = static_cast<uint64_t>(ticks.QuadPart);
    const auto f = static_cast<uint64_t>(frequency.QuadPart);
    if (t / f > (std::numeric_limits<uint64_t>::max() - 10000000ULL) / 10000000ULL) return E_FAIL;
    value = (t / f) * 10000000ULL + ((t % f) * 10000000ULL) / f;
    return S_OK;
}
class WindowsBackend final : public Backend {
    DWORD owner_ = GetCurrentThreadId();
    std::unique_ptr<ComScope> com_;
    ComPtr<IAudioClient> client_;
    ComPtr<IAudioRenderClient> render_;
    ComPtr<IAudioClock> clock_;
    HANDLE event_ = nullptr;
    Format format_;
    bool initialized_ = false, ever_prepared_ = false;
    HRESULT prepare_inner(const Pair& pair, uint32_t channels, Format& out) {
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        // No new generation on this backend until external drain can be proved.
        if (ever_prepared_) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
        if (channels != 1 && channels != 2) return E_INVALIDARG;
        com_ = std::make_unique<ComScope>();
        if (FAILED(com_->hr)) return com_->hr;
        std::vector<Endpoint> endpoints;
        HRESULT hr = enumerate_on_mta(endpoints);
        if (FAILED(hr)) return hr;
        hr = validate_pair(endpoints, pair);
        if (FAILED(hr)) return hr; // A caller cannot spoof metadata to open speakers.
        ComPtr<IMMDeviceEnumerator> enumerator;
        hr = CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_INPROC_SERVER,
                              IID_PPV_ARGS(&enumerator));
        if (FAILED(hr)) return hr;
        ComPtr<IMMDevice> device;
        hr = enumerator->GetDevice(pair.render_id.c_str(), &device);
        if (FAILED(hr)) return hr;
        ever_prepared_ = true;
        hr = device->Activate(__uuidof(IAudioClient), CLSCTX_INPROC_SERVER, nullptr, &client_);
        if (FAILED(hr)) return hr;
        WAVEFORMATEX wave{};
        wave.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
        wave.nChannels = static_cast<WORD>(channels);
        wave.nSamplesPerSec = sample_rate;
        wave.wBitsPerSample = 32;
        wave.nBlockAlign = static_cast<WORD>(channels * sizeof(float));
        wave.nAvgBytesPerSec = wave.nSamplesPerSec * wave.nBlockAlign;
        WAVEFORMATEX* closest = nullptr;
        hr = client_->IsFormatSupported(AUDCLNT_SHAREMODE_SHARED, &wave, &closest);
        CoTaskMemFree(closest);
        if (hr != S_OK) return FAILED(hr) ? hr : AUDCLNT_E_UNSUPPORTED_FORMAT;
        hr = client_->Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                                 200000, 0, &wave, nullptr);
        if (FAILED(hr)) return hr;
        initialized_ = true;
        format_.rate = sample_rate;
        format_.channels = channels;
        hr = client_->GetBufferSize(&format_.buffer_frames);
        if (FAILED(hr)) return hr;
        if (format_.buffer_frames < max_write_frames || format_.buffer_frames > max_buffer_frames)
            return HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED);
        event_ = CreateEventW(nullptr, FALSE, FALSE, nullptr);
        if (!event_) return HRESULT_FROM_WIN32(GetLastError());
        hr = client_->SetEventHandle(event_);
        if (FAILED(hr)) return hr;
        hr = client_->GetService(IID_PPV_ARGS(&render_));
        if (FAILED(hr)) return hr;
        hr = client_->GetService(IID_PPV_ARGS(&clock_));
        if (FAILED(hr)) return hr;
        hr = clock_->GetFrequency(&format_.clock_frequency);
        if (FAILED(hr)) return hr;
        if (!format_.clock_frequency || format_.clock_frequency > 1000000000ULL) return E_UNEXPECTED;
        out = format_;
        return S_OK;
    }
public:
    ~WindowsBackend() override {
        stop_reset();
        clock_.Reset(); render_.Reset(); client_.Reset();
        if (event_) CloseHandle(event_);
        // COM is uninitialized last by com_. All destruction is on the owner MTA.
    }
    HRESULT prepare(const Pair& pair, uint32_t channels, Format& out) noexcept override {
        out = {};
        try { return prepare_inner(pair, channels, out); }
        catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
        catch (...) { return E_FAIL; }
    }
    HRESULT start() noexcept override {
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        return initialized_ && render_ && clock_ ? client_->Start() : E_UNEXPECTED;
    }
    HRESULT snapshot(Snapshot& out) noexcept override {
        out = {};
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        if (!initialized_ || !clock_) return E_UNEXPECTED;
        HRESULT hr = client_->GetCurrentPadding(&out.padding);
        if (FAILED(hr)) return hr;
        hr = clock_->GetPosition(&out.position, &out.qpc_100ns);
        if (hr != S_OK) return FAILED(hr) ? hr : HRESULT_FROM_WIN32(ERROR_RETRY);
        uint64_t now = 0;
        hr = current_qpc(now);
        if (FAILED(hr)) return hr;
        // Clock timestamps are already in 100 ns QPC units. Reject stale/future
        // snapshots instead of manufacturing a presentation time from wall time.
        if (out.qpc_100ns > now || now - out.qpc_100ns > 50000)
            return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        return out.padding <= format_.buffer_frames ? S_OK : E_UNEXPECTED;
    }
    HRESULT write_mono(const float* mono, uint32_t frames, bool silence) noexcept override {
        if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
        if (!render_ || !frames || frames > max_write_frames || (!silence && !mono)) return E_INVALIDARG;
        if (!silence) for (uint32_t i = 0; i < frames; ++i)
            if (!std::isfinite(mono[i]) || std::abs(mono[i]) > 1.0f) return E_INVALIDARG;
        UINT32 padding = 0;
        HRESULT hr = client_->GetCurrentPadding(&padding);
        if (FAILED(hr)) return hr;
        if (padding > format_.buffer_frames || frames > format_.buffer_frames - padding)
            return AUDCLNT_E_BUFFER_TOO_LARGE;
        BYTE* data = nullptr;
        hr = render_->GetBuffer(frames, &data);
        if (FAILED(hr)) return hr;
        if (!silence) {
            auto output = reinterpret_cast<float*>(data);
            hr = copy_mono_interleaved(mono, frames, format_.channels, output, frames * format_.channels);
            if (FAILED(hr)) { render_->ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT); return hr; }
        }
        return render_->ReleaseBuffer(frames, silence ? AUDCLNT_BUFFERFLAGS_SILENT : 0);
    }
    Cleanup stop_reset() noexcept override {
        Cleanup result;
        if (GetCurrentThreadId() != owner_) { result.stop_error = RPC_E_WRONG_THREAD; return result; }
        if (!initialized_) { result.client_reset = true; return result; }
        result.stop_error = client_->Stop();
        result.reset_error = client_->Reset(); // Attempt even when Stop failed; report both.
        result.client_reset = SUCCEEDED(result.stop_error) && SUCCEEDED(result.reset_error);
        return result;
    }
    HANDLE ready_event() const noexcept override { return event_; }
};
}
HRESULT enumerate_endpoints(std::vector<Endpoint>& endpoints) noexcept {
    endpoints.clear();
    try {
        ComScope com;
        if (FAILED(com.hr)) return com.hr;
        return enumerate_on_mta(endpoints);
    } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
    catch (...) { return E_FAIL; }
}
std::unique_ptr<Backend> make_windows_backend() { return std::make_unique<WindowsBackend>(); }
}
