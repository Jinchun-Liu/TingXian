#include "input_frame.h"
#include "video_timing.h"
#include <mfapi.h>
#include <mferror.h>
#include <wrl/client.h>
#include <new>
#include <utility>
#include <limits>

namespace tingxian::camera_input {
using Microsoft::WRL::ComPtr;
void Frame::clear() noexcept {
    if (!bgr.empty()) SecureZeroMemory(bgr.data(), bgr.size());
    bgr.clear(); generation = sequence = capture_qpc = 0; discontinuity = 0;
}
Frame::Frame(Frame&& other) noexcept { *this = std::move(other); }
Frame& Frame::operator=(Frame&& other) noexcept {
    if (this != &other) {
        clear(); generation = other.generation; sequence = other.sequence; capture_qpc = other.capture_qpc;
        discontinuity = other.discontinuity;
        bgr = std::move(other.bgr); other.clear();
    }
    return *this;
}
HRESULT copy_bgr(const BYTE* start, size_t length, size_t offset, LONG pitch, Frame& out) {
    out.clear();
    const auto stride = static_cast<INT64>(pitch);
    if (!start || length > 32 * 1024 * 1024 || offset > length ||
        (stride < 0 ? -stride : stride) < Width * 4) return MF_E_INVALIDMEDIATYPE;
    const auto first = static_cast<INT64>(offset), last = first + stride * (Height - 1);
    if (first < 0 || last < 0 || first + Width * 4 > static_cast<INT64>(length) ||
        last + Width * 4 > static_cast<INT64>(length)) return MF_E_BUFFERTOOSMALL;
    out.bgr.resize(BgrBytes);
    for (UINT32 y = 0; y < Height; ++y) {
        const auto* row = start + size_t(first + stride * y);
        auto* dst = out.bgr.data() + y * Width * 3;
        for (UINT32 x = 0; x < Width; ++x) {
            dst[x * 3] = row[x * 4]; dst[x * 3 + 1] = row[x * 4 + 1]; dst[x * 3 + 2] = row[x * 4 + 2];
        }
    }
    return S_OK;
}
HRESULT decode(IMFSample* sample, LONG stride, Frame& out) {
    out.clear(); if (!sample) return E_POINTER;
    UINT64 qpc = 0;
    auto hr = sample->GetUINT64(MFSampleExtension_DeviceTimestamp, &qpc);
    if (FAILED(hr) || !qpc || qpc > UINT64(LLONG_MAX)) return MF_E_NO_SAMPLE_TIMESTAMP;
    UINT32 discontinuity = 0;
    hr = sample->GetUINT32(MFSampleExtension_Discontinuity, &discontinuity);
    if ((FAILED(hr) && hr != MF_E_ATTRIBUTENOTFOUND) || discontinuity > 1) return MF_E_INVALID_STREAM_DATA;
    // A missing video frame is admissible; time must still be fresh and strictly
    // increasing. Preserve a discontinuity in status instead of inventing frames.
    DWORD buffers = 0; hr = sample->GetBufferCount(&buffers);
    if (FAILED(hr)) return hr;
    if (buffers != 1) return MF_E_INVALID_STREAM_DATA;
    ComPtr<IMFMediaBuffer> buffer; hr = sample->GetBufferByIndex(0, &buffer);
    if (FAILED(hr)) return hr;
    ComPtr<IMF2DBuffer2> two;
    hr = buffer.As(&two);
    BYTE *start = nullptr, *top = nullptr; DWORD length = 0;
    HRESULT result = E_FAIL, released = S_OK;
    if (SUCCEEDED(hr)) {
        LONG pitch = 0; hr = two->Lock2DSize(MF2DBuffer_LockFlags_Read, &top, &pitch, &start, &length);
        if (FAILED(hr)) return hr;
        // Validate integer address difference before doing pointer arithmetic.
        const auto a = reinterpret_cast<uintptr_t>(start), b = reinterpret_cast<uintptr_t>(top);
        try { result = b >= a && b - a <= length ? copy_bgr(start, length, b - a, pitch, out) : MF_E_BUFFERTOOSMALL; }
        catch (...) { result = E_OUTOFMEMORY; }
        released = two->Unlock2D();
    } else if (hr == E_NOINTERFACE) {
        DWORD capacity = 0; hr = buffer->Lock(&start, &capacity, &length);
        if (FAILED(hr)) return hr;
        const auto offset = stride < 0 ? size_t(-static_cast<INT64>(stride)) * (Height - 1) : 0;
        try { result = length <= capacity ? copy_bgr(start, length, offset, stride, out) : MF_E_BUFFERTOOSMALL; }
        catch (...) { result = E_OUTOFMEMORY; }
        released = buffer->Unlock();
    } else return hr;
    if (FAILED(released)) result = released;
    if (SUCCEEDED(result)) { out.capture_qpc = qpc; out.discontinuity = discontinuity; } else out.clear();
    return result;
}
Mailbox::Mailbox(UINT64 generation) : generation_(generation) {
    ready_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    flushed_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    retired_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
}
Mailbox::~Mailbox() {
    close(); if (ready_) CloseHandle(ready_); if (flushed_) CloseHandle(flushed_); if (retired_) CloseHandle(retired_);
}
bool Mailbox::valid() const noexcept { return generation_ && ready_ && flushed_ && retired_; }
void Mailbox::clear_locked() noexcept {
    if (!frame_.bgr.empty()) { ++counts_.cleared; frame_.clear(); }
    counts_.queued = 0;
}
void Mailbox::fault(HRESULT hr) noexcept {
    closed_.store(true);
    std::lock_guard<std::mutex> lock(mutex_);
    if (SUCCEEDED(counts_.error)) counts_.error = FAILED(hr) ? hr : E_UNEXPECTED;
    clear_locked(); SetEvent(ready_);
}
void Mailbox::sample(HRESULT hr, DWORD stream, DWORD flags, IMFSample* sample, LONG stride, UINT64 now) noexcept {
    std::lock_guard<std::mutex> lock(mutex_);
    if (closed_.load()) { ++counts_.late; return; }
    const auto fail = [&](HRESULT error) { counts_.error = error; closed_.store(true); clear_locked(); SetEvent(ready_); };
    // No auto format change, hidden second stream, EOS, timestamp substitution
    // or retry after device/permission error. STREAMTICK alone is a gap, not a frame.
    if (FAILED(hr)) { fail(hr); return; }
    if (stream != 0 || (flags & ~DWORD(MF_SOURCE_READERF_STREAMTICK))) { fail(MF_E_INVALID_STREAM_DATA); return; }
    if (!sample && flags == MF_SOURCE_READERF_STREAMTICK) { SetEvent(ready_); return; }
    if (flags) { fail(MF_E_INVALID_STREAM_DATA); return; }
    Frame incoming;
    try { hr = decode(sample, stride, incoming); } catch (...) { hr = E_OUTOFMEMORY; }
    if (FAILED(hr)) { fail(hr); return; }
    if (!now || incoming.capture_qpc > now || incoming.capture_qpc <= previous_) { fail(MF_E_INVALID_TIMESTAMP); return; }
    previous_ = incoming.capture_qpc;
    if (incoming.discontinuity) ++counts_.discontinuities;
    if (now - incoming.capture_qpc > MaxAge) { ++counts_.stale; SetEvent(ready_); return; }
    if (closed_.load()) { ++counts_.late; return; }
    if (sequence_ == UINT64_MAX) { fail(HRESULT_FROM_WIN32(ERROR_ARITHMETIC_OVERFLOW)); return; }
    incoming.generation = generation_; incoming.sequence = ++sequence_;
    if (!frame_.bgr.empty()) ++counts_.superseded;
    frame_ = std::move(incoming); ++counts_.accepted; counts_.queued = 1; SetEvent(ready_);
}
HRESULT Mailbox::take(UINT64 generation, UINT64 now, Frame& out) noexcept {
    out.clear(); std::lock_guard<std::mutex> lock(mutex_);
    if (generation != generation_) return E_INVALIDARG;
    if (FAILED(counts_.error)) return counts_.error;
    if (closed_.load()) return MF_E_SHUTDOWN;
    ResetEvent(ready_);
    if (frame_.bgr.empty()) return S_FALSE;
    if (!now || frame_.capture_qpc > now) { closed_.store(true); counts_.error = MF_E_INVALID_TIMESTAMP; clear_locked(); SetEvent(ready_); return counts_.error; }
    if (now - frame_.capture_qpc > MaxAge) { ++counts_.stale; frame_.clear(); counts_.queued = 0; return S_FALSE; }
    out = std::move(frame_); counts_.queued = 0; ++counts_.delivered; return S_OK;
}
void Mailbox::close() noexcept {
    closed_.store(true); std::lock_guard<std::mutex> lock(mutex_); clear_locked(); if (ready_) SetEvent(ready_);
}
Counts Mailbox::counts() const noexcept {
    std::lock_guard<std::mutex> lock(mutex_); auto result = counts_; result.closed = closed_.load(); return result;
}
namespace {
class Callback final : public IMFSourceReaderCallback {
    std::atomic<ULONG> refs_{1};
    std::shared_ptr<Mailbox> mailbox_;
    LONG stride_;
    ~Callback() { SetEvent(mailbox_->retired()); }
public:
    Callback(std::shared_ptr<Mailbox> m, LONG stride) : mailbox_(std::move(m)), stride_(stride) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** out) override {
        if (!out) return E_POINTER; *out = nullptr;
        if (iid != __uuidof(IUnknown) && iid != __uuidof(IMFSourceReaderCallback)) return E_NOINTERFACE;
        *out = static_cast<IMFSourceReaderCallback*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs_; }
    ULONG STDMETHODCALLTYPE Release() override { const auto n = --refs_; if (!n) delete this; return n; }
    HRESULT STDMETHODCALLTYPE OnReadSample(HRESULT hr, DWORD stream, DWORD flags, LONGLONG, IMFSample* sample) override {
        UINT64 now = 0; video::qpc_now(now); mailbox_->sample(hr, stream, flags, sample, stride_, now); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE OnFlush(DWORD stream) override {
        if (stream != 0 && stream != DWORD(MF_SOURCE_READER_ALL_STREAMS)) mailbox_->fault(MF_E_INVALIDSTREAMNUMBER);
        SetEvent(mailbox_->flushed()); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE OnEvent(DWORD, IMFMediaEvent* event) override {
        HRESULT status = E_UNEXPECTED;
        if (!event || FAILED(event->GetStatus(&status))) mailbox_->fault(E_UNEXPECTED);
        else if (FAILED(status)) mailbox_->fault(status);
        return S_OK;
    }
};
}
HRESULT make_callback(const std::shared_ptr<Mailbox>& mailbox, LONG stride, IMFSourceReaderCallback** out) {
    if (!out) return E_POINTER; *out = nullptr;
    if (!mailbox || !mailbox->valid()) return E_INVALIDARG;
    // MF can release the final callback reference on a system worker. Its
    // destructor event precedes return from Release, so that event alone does
    // NOT permit DLL unload. Pin only this small optional code module for the
    // process lifetime before handing any callback to MF; devices/MF sessions
    // and frame buffers are still explicitly retired. No vendor SDK is pinned.
    static const HRESULT code_pin = [] {
        HMODULE module = nullptr;
        return GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            reinterpret_cast<LPCWSTR>(&make_callback), &module) ? S_OK : HRESULT_FROM_WIN32(GetLastError());
    }();
    if (FAILED(code_pin)) return code_pin;
    *out = new (std::nothrow) Callback(mailbox, stride); return *out ? S_OK : E_OUTOFMEMORY;
}
}
