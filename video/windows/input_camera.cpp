#include "input_camera.h"
#include "video_timing.h"
#include <mfapi.h>
#include <mferror.h>
#include <wrl/client.h>
#include <algorithm>

namespace tingxian::camera_input {
using Microsoft::WRL::ComPtr;
namespace {
constexpr DWORD CleanupWaitMs = 500;
struct Activations {
    IMFActivate** list = nullptr; UINT32 count = 0;
    ~Activations() { for (UINT32 i = 0; i < count; ++i) list[i]->Release(); CoTaskMemFree(list); }
};
struct String { wchar_t* p = nullptr; ~String() { CoTaskMemFree(p); } };
HRESULT selected_hardware(const std::wstring& id, IMFMediaSource** out) {
    ComPtr<IMFAttributes> attributes;
    auto hr = MFCreateAttributes(&attributes, 1); if (FAILED(hr)) return hr;
    hr = attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
    if (FAILED(hr)) return hr;
    Activations devices; hr = MFEnumDeviceSources(attributes.Get(), &devices.list, &devices.count);
    if (FAILED(hr)) return hr;
    if (devices.count > 256) return HRESULT_FROM_WIN32(ERROR_TOO_MANY_NAMES);
    for (UINT32 i = 0; i < devices.count; ++i) {
        String link; UINT32 length = 0;
        hr = devices.list[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &link.p, &length);
        if (FAILED(hr)) return hr;
        if (length != id.size() || CompareStringOrdinal(link.p, length, id.data(), static_cast<int>(id.size()), TRUE) != CSTR_EQUAL) continue;
        UINT32 hardware = 0;
        hr = devices.list[i]->GetUINT32(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_HW_SOURCE, &hardware);
        // Initial supported boundary: physical hardware only. Software cameras
        // (including our own return path) need a separate explicit source policy.
        if (FAILED(hr) || hardware != 1) return HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED);
        return devices.list[i]->ActivateObject(__uuidof(IMFMediaSource), reinterpret_cast<void**>(out));
    }
    return HRESULT_FROM_WIN32(ERROR_NOT_FOUND); // never fall back to first/default
}
bool exact_format(IMFMediaType* t) noexcept {
    UINT32 w = 0, h = 0, n = 0, d = 0, interlace = 0; GUID major{};
    return SUCCEEDED(t->GetGUID(MF_MT_MAJOR_TYPE, &major)) && major == MFMediaType_Video &&
        SUCCEEDED(MFGetAttributeSize(t, MF_MT_FRAME_SIZE, &w, &h)) && w == Width && h == Height &&
        SUCCEEDED(MFGetAttributeRatio(t, MF_MT_FRAME_RATE, &n, &d)) && d && UINT64(n) == UINT64(d) * 30 &&
        SUCCEEDED(t->GetUINT32(MF_MT_INTERLACE_MODE, &interlace)) && interlace == MFVideoInterlace_Progressive;
}
HRESULT configure(IMFSourceReader* reader) {
    auto hr = reader->SetStreamSelection(DWORD(MF_SOURCE_READER_ALL_STREAMS), FALSE); if (FAILED(hr)) return hr;
    hr = reader->SetStreamSelection(0, TRUE); if (FAILED(hr)) return hr;
    bool selected = false;
    for (DWORD i = 0; i < 256; ++i) {
        ComPtr<IMFMediaType> native;
        hr = reader->GetNativeMediaType(0, i, &native);
        if (hr == MF_E_NO_MORE_TYPES) break;
        if (FAILED(hr)) return hr;
        if (exact_format(native.Get())) {
            hr = reader->SetCurrentMediaType(0, nullptr, native.Get());
            if (FAILED(hr)) return hr;
            selected = true; break;
        }
    }
    if (!selected) return MF_E_INVALIDMEDIATYPE;
    ComPtr<IMFMediaType> output; hr = MFCreateMediaType(&output); if (FAILED(hr)) return hr;
    if (FAILED(hr = output->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video)) ||
        FAILED(hr = output->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_RGB32)) ||
        FAILED(hr = MFSetAttributeSize(output.Get(), MF_MT_FRAME_SIZE, Width, Height)) ||
        FAILED(hr = MFSetAttributeRatio(output.Get(), MF_MT_FRAME_RATE, 30, 1)) ||
        FAILED(hr = output->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive)) ||
        FAILED(hr = output->SetUINT32(MF_MT_DEFAULT_STRIDE, Width * 4))) return hr;
    hr = reader->SetCurrentMediaType(0, nullptr, output.Get()); if (FAILED(hr)) return hr;
    ComPtr<IMFMediaType> actual; hr = reader->GetCurrentMediaType(0, &actual); if (FAILED(hr)) return hr;
    GUID subtype{}; UINT32 stride = 0;
    if (!exact_format(actual.Get()) || FAILED(actual->GetGUID(MF_MT_SUBTYPE, &subtype)) || subtype != MFVideoFormat_RGB32 ||
        FAILED(actual->GetUINT32(MF_MT_DEFAULT_STRIDE, &stride)) || stride != Width * 4) return MF_E_INVALIDMEDIATYPE;
    return S_OK;
}
}
struct CameraInput::Impl {
    const DWORD thread = GetCurrentThreadId();
    Status state;
    std::shared_ptr<Mailbox> mailbox;
    ComPtr<IMFMediaSource> source;
    ComPtr<IMFSourceReader> reader;
    ComPtr<IMFSourceReaderCallback> callback;
    bool mf = false, requested = false;
    UINT64 generation = 0;
    HRESULT correct_thread() const noexcept { return thread == GetCurrentThreadId() ? S_OK : RPC_E_WRONG_THREAD; }
    HRESULT failure(HRESULT hr) noexcept { state.operation = hr; state.phase = Phase::fault; if (mailbox) mailbox->fault(hr); return hr; }
    HRESULT request() noexcept {
        auto hr = reader->ReadSample(0, 0, nullptr, nullptr, nullptr, nullptr);
        if (FAILED(hr)) return failure(hr);
        requested = true; return S_OK;
    }
};
CameraInput::CameraInput() : impl_(std::make_unique<Impl>()) {}
CameraInput::~CameraInput() {
    // Never free callback state or unload MF under uncertain live callbacks.
    // Host must inspect stop/status; an uncertain instance cannot be restarted.
    if (FAILED(stop()) && (impl_->state.cleanup_uncertain || FAILED(impl_->correct_thread()))) (void)impl_.release();
}
HRESULT CameraInput::prepare(const std::wstring& link, UINT64 generation, bool intent) noexcept {
    auto& s = *impl_;
    if (FAILED(s.correct_thread())) return RPC_E_WRONG_THREAD;
    if (!intent) return E_ACCESSDENIED; // before COM, enumeration or activation
    if (s.state.phase != Phase::idle) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    if (link.empty() || link.size() > 32767 || link.find(L'\0') != std::wstring::npos || !generation) return E_INVALIDARG;
    s.state.phase = Phase::preparing; s.generation = generation;
    APTTYPE type; APTTYPEQUALIFIER qualifier;
    auto hr = CoGetApartmentType(&type, &qualifier);
    if (FAILED(hr)) return s.failure(hr);
    if (type != APTTYPE_MTA) return s.failure(RPC_E_WRONG_THREAD);
    try {
        s.mailbox = std::make_shared<Mailbox>(generation);
        if (!s.mailbox->valid()) return s.failure(E_OUTOFMEMORY);
        hr = MFStartup(MF_VERSION, MFSTARTUP_FULL); if (FAILED(hr)) return s.failure(hr);
        s.mf = true;
        hr = selected_hardware(link, &s.source); if (FAILED(hr)) return s.failure(hr);
        hr = make_callback(s.mailbox, LONG(Width * 4), &s.callback); if (FAILED(hr)) return s.failure(hr);
        ComPtr<IMFAttributes> attributes;
        if (FAILED(hr = MFCreateAttributes(&attributes, 4)) ||
            FAILED(hr = attributes->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, s.callback.Get())) ||
            FAILED(hr = attributes->SetUINT32(MF_LOW_LATENCY, TRUE)) ||
            FAILED(hr = attributes->SetUINT32(MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, TRUE)) ||
            FAILED(hr = attributes->SetUINT32(MF_SOURCE_READER_DISCONNECT_MEDIASOURCE_ON_SHUTDOWN, TRUE)) ||
            FAILED(hr = MFCreateSourceReaderFromMediaSource(s.source.Get(), attributes.Get(), &s.reader)) ||
            FAILED(hr = configure(s.reader.Get()))) return s.failure(hr);
        // No ReadSample before a separate explicit start. Preparation may open
        // the selected device, hence it is also gated by explicit user intent.
        s.state.phase = Phase::ready; return S_OK;
    } catch (const std::bad_alloc&) { return s.failure(E_OUTOFMEMORY); }
    catch (...) { return s.failure(E_FAIL); }
}
HRESULT CameraInput::start() noexcept {
    auto& s = *impl_; if (FAILED(s.correct_thread())) return RPC_E_WRONG_THREAD;
    if (s.state.phase != Phase::ready) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    s.state.phase = Phase::running; return s.request();
}
HRESULT CameraInput::read(Frame& out) noexcept {
    out.clear(); auto& s = *impl_; if (FAILED(s.correct_thread())) return RPC_E_WRONG_THREAD;
    if (s.state.phase != Phase::running) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    const auto wait = WaitForSingleObject(s.mailbox->ready(), 0);
    if (wait == WAIT_TIMEOUT) return S_FALSE;
    if (wait != WAIT_OBJECT_0) return s.failure(HRESULT_FROM_WIN32(GetLastError()));
    UINT64 now = 0; if (!video::qpc_now(now)) return s.failure(E_UNEXPECTED);
    auto hr = s.mailbox->take(s.generation, now, out);
    if (FAILED(hr)) return s.failure(hr);
    s.requested = false;
    // Only owner requests the next read: one outstanding read + one copied
    // frame at most. No callback-reader reference cycle or unbounded FIFO.
    auto next = s.request();
    if (FAILED(next)) { out.clear(); return next; }
    return hr;
}
HRESULT CameraInput::stop() noexcept {
    auto& s = *impl_; if (FAILED(s.correct_thread())) return RPC_E_WRONG_THREAD;
    if (s.state.phase == Phase::stopped) return S_OK;
    if (s.state.cleanup_uncertain) return HRESULT_FROM_WIN32(ERROR_BUSY);
    s.state.phase = Phase::stopping;
    if (s.mailbox) s.mailbox->close(); // revoke first, including callbacks still copying
    if (s.reader && s.requested) {
        s.state.flush = s.reader->Flush(DWORD(MF_SOURCE_READER_ALL_STREAMS));
        if (SUCCEEDED(s.state.flush) && WaitForSingleObject(s.mailbox->flushed(), CleanupWaitMs) != WAIT_OBJECT_0)
            s.state.flush = HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    }
    if (s.source) s.state.shutdown = s.source->Shutdown();
    const bool had_callback = bool(s.callback);
    s.reader.Reset(); s.source.Reset(); s.callback.Reset();
    if (had_callback && WaitForSingleObject(s.mailbox->retired(), CleanupWaitMs) != WAIT_OBJECT_0)
        s.state.release = HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    if (FAILED(s.state.flush) || FAILED(s.state.shutdown) || FAILED(s.state.release)) {
        s.state.cleanup_uncertain = true; s.state.phase = Phase::fault;
        return FAILED(s.state.flush) ? s.state.flush : FAILED(s.state.shutdown) ? s.state.shutdown : s.state.release;
    }
    if (s.mf) {
        s.state.release = MFShutdown();
        if (FAILED(s.state.release)) { s.state.cleanup_uncertain = true; s.state.phase = Phase::fault; return s.state.release; }
        s.mf = false;
    }
    s.state.phase = Phase::stopped; return S_OK;
}
Status CameraInput::status() const noexcept {
    const auto& s = *impl_;
    if (FAILED(s.correct_thread())) { Status result; result.phase = Phase::fault; result.operation = RPC_E_WRONG_THREAD; return result; }
    auto result = s.state;
    if (s.mailbox) {
        result.frames = s.mailbox->counts();
        if (FAILED(result.frames.error) && result.phase != Phase::stopped && result.phase != Phase::stopping) {
            result.phase = Phase::fault;
            if (SUCCEEDED(result.operation)) result.operation = result.frames.error;
        }
    }
    return result;
}
HANDLE CameraInput::ready_event() const noexcept {
    const auto& s = *impl_; return SUCCEEDED(s.correct_thread()) && s.mailbox ? s.mailbox->ready() : nullptr;
}
}
