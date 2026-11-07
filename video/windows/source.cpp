// Media Foundation contract follows Microsoft's MIT-licensed VirtualCamera sample.
// See THIRD-PARTY.md and LICENSE-Microsoft.txt. TingXian supplies only NV12 frames;
// there is no physical-camera wrapper, network client, GPU SDK or registration code.
#include "frame_mailbox.h"
#include "frame_transport.h"
#include <mfapi.h>
#include <mfidl.h>
#include <mferror.h>
#include <initguid.h>
#include <ks.h>
#include <ksmedia.h>
#include <ksproxy.h>
#include <propvarutil.h>
#include <wrl.h>
#include <atomic>
#include <memory>
#include <new>

using Microsoft::WRL::ComPtr;
using Microsoft::WRL::RuntimeClass;
using Microsoft::WRL::RuntimeClassFlags;
using Microsoft::WRL::ClassicCom;
using Microsoft::WRL::ChainInterfaces;
using Microsoft::WRL::Make;
using namespace tingxian::video;

#define TX_CHECK(expression) do { const HRESULT tx_hr = (expression); if (FAILED(tx_hr)) return tx_hr; } while (false)

namespace {
std::atomic<long> objects{0};
std::atomic<long> server_locks{0};
struct Tracked {
    Tracked() { ++objects; }
    ~Tracked() { --objects; }
};

// Copy the queue under the state lock, then release it before a blocking GetEvent.
#define TX_EVENTS \
    STDMETHODIMP GetEvent(DWORD flags, IMFMediaEvent** event) override { \
        ComPtr<IMFMediaEventQueue> q; TX_CHECK(event_queue(&q)); return q->GetEvent(flags, event); } \
    STDMETHODIMP BeginGetEvent(IMFAsyncCallback* callback, IUnknown* state) override { \
        ComPtr<IMFMediaEventQueue> q; TX_CHECK(event_queue(&q)); return q->BeginGetEvent(callback, state); } \
    STDMETHODIMP EndGetEvent(IMFAsyncResult* result, IMFMediaEvent** event) override { \
        ComPtr<IMFMediaEventQueue> q; TX_CHECK(event_queue(&q)); return q->EndGetEvent(result, event); } \
    STDMETHODIMP QueueEvent(MediaEventType type, REFGUID extended, HRESULT status, const PROPVARIANT* value) override { \
        ComPtr<IMFMediaEventQueue> q; TX_CHECK(event_queue(&q)); return q->QueueEventParamVar(type, extended, status, value); }

class Stream final : public RuntimeClass<RuntimeClassFlags<ClassicCom>,
    ChainInterfaces<IMFMediaStream2, IMFMediaStream, IMFMediaEventGenerator>>, public Tracked {
    std::mutex lock_;
    bool shutdown_ = false;
    bool allocator_ready_ = false;
    MF_STREAM_STATE state_ = MF_STREAM_STATE_STOPPED;
    LONGLONG last_time_ = 0;
    ComPtr<IMFMediaSource> parent_;
    ComPtr<IMFMediaEventQueue> events_;
    ComPtr<IMFAttributes> attributes_;
    ComPtr<IMFStreamDescriptor> descriptor_;
    ComPtr<IMFMediaType> type_;
    ComPtr<IMFVideoSampleAllocator> allocator_;
    std::shared_ptr<FrameMailbox> mailbox_;
    HRESULT event_queue(IMFMediaEventQueue** output) {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : events_.CopyTo(output);
    }
public:
    HRESULT initialize(IMFMediaSource* parent, std::shared_ptr<FrameMailbox> mailbox) {
        parent_ = parent;
        mailbox_ = std::move(mailbox);
        TX_CHECK(MFCreateEventQueue(&events_));
        TX_CHECK(MFCreateMediaType(&type_));
        TX_CHECK(type_->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video));
        TX_CHECK(type_->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12));
        TX_CHECK(type_->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive));
        TX_CHECK(type_->SetUINT32(MF_MT_ALL_SAMPLES_INDEPENDENT, TRUE));
        TX_CHECK(MFSetAttributeSize(type_.Get(), MF_MT_FRAME_SIZE, Width, Height));
        TX_CHECK(MFSetAttributeRatio(type_.Get(), MF_MT_FRAME_RATE, 30, 1));
        TX_CHECK(MFSetAttributeRatio(type_.Get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1));
        TX_CHECK(type_->SetUINT32(MF_MT_AVG_BITRATE, FrameBytes * 8 * 30));
        TX_CHECK(type_->SetUINT32(MF_MT_DEFAULT_STRIDE, Width));
        TX_CHECK(type_->SetUINT32(MF_MT_SAMPLE_SIZE, FrameBytes));
        TX_CHECK(type_->SetUINT32(MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235));
        TX_CHECK(type_->SetUINT32(MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709));
        TX_CHECK(type_->SetUINT32(MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709));
        TX_CHECK(type_->SetUINT32(MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709));
        TX_CHECK(type_->SetUINT32(MF_MT_VIDEO_CHROMA_SITING,
            MFVideoChromaSubsampling_ProgressiveChroma | MFVideoChromaSubsampling_MPEG1));
        IMFMediaType* types[] = {type_.Get()};
        TX_CHECK(MFCreateStreamDescriptor(0, 1, types, &descriptor_));
        ComPtr<IMFMediaTypeHandler> handler;
        TX_CHECK(descriptor_->GetMediaTypeHandler(&handler));
        TX_CHECK(handler->SetCurrentMediaType(type_.Get()));
        TX_CHECK(MFCreateAttributes(&attributes_, 4));
        TX_CHECK(attributes_->SetGUID(MF_DEVICESTREAM_STREAM_CATEGORY, PINNAME_VIDEO_CAPTURE));
        TX_CHECK(attributes_->SetUINT32(MF_DEVICESTREAM_STREAM_ID, 0));
        TX_CHECK(attributes_->SetUINT32(MF_DEVICESTREAM_FRAMESERVER_SHARED, 1));
        TX_CHECK(attributes_->SetUINT32(MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color));
        return attributes_->CopyAllItems(descriptor_.Get());
    }
    TX_EVENTS
    STDMETHODIMP GetMediaSource(IMFMediaSource** source) override {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : parent_.CopyTo(source);
    }
    STDMETHODIMP GetStreamDescriptor(IMFStreamDescriptor** descriptor) override {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : descriptor_.CopyTo(descriptor);
    }
    HRESULT attributes(IMFAttributes** attributes) {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : attributes_.CopyTo(attributes);
    }
    HRESULT set_allocator(IUnknown* allocator) {
        if (!allocator) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        if (state_ != MF_STREAM_STATE_STOPPED) return MF_E_INVALIDREQUEST;
        ComPtr<IMFVideoSampleAllocator> next;
        TX_CHECK(allocator->QueryInterface(IID_PPV_ARGS(&next)));
        if (allocator_ready_) { allocator_->UninitializeSampleAllocator(); allocator_ready_ = false; }
        allocator_ = next;
        return S_OK;
    }
    HRESULT start(IMFMediaType* type, bool send_event = true) {
        if (!type) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        DWORD equal_flags = 0;
        if (type_->IsEqual(type, &equal_flags) != S_OK) return MF_E_INVALIDMEDIATYPE;
        if (!allocator_) return MF_E_NOT_INITIALIZED;
        if (!allocator_ready_) {
            // Four reusable output samples are distinct from the one pending input frame.
            TX_CHECK(allocator_->InitializeSampleAllocator(4, type_.Get()));
            allocator_ready_ = true;
        }
        if (send_event) TX_CHECK(events_->QueueEventParamVar(MEStreamStarted, GUID_NULL, S_OK, nullptr));
        state_ = MF_STREAM_STATE_RUNNING;
        return S_OK;
    }
    HRESULT stop(bool send_event = true) {
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        state_ = MF_STREAM_STATE_STOPPED;
        mailbox_->stop();
        if (allocator_ready_) { allocator_->UninitializeSampleAllocator(); allocator_ready_ = false; }
        return send_event ? events_->QueueEventParamVar(MEStreamStopped, GUID_NULL, S_OK, nullptr) : S_OK;
    }
    void shutdown() {
        std::lock_guard guard(lock_);
        if (shutdown_) return;
        shutdown_ = true;
        mailbox_->stop();
        if (events_) events_->Shutdown();
        if (allocator_ready_) allocator_->UninitializeSampleAllocator();
        allocator_ready_ = false;
        parent_.Reset(); allocator_.Reset(); events_.Reset(); descriptor_.Reset(); attributes_.Reset(); type_.Reset();
    }
    STDMETHODIMP RequestSample(IUnknown* token) override {
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        if (state_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALIDREQUEST;
        ComPtr<IMFSample> sample;
        TX_CHECK(allocator_->AllocateSample(&sample));
        ComPtr<IMFMediaBuffer> buffer;
        TX_CHECK(sample->GetBufferByIndex(0, &buffer));
        ComPtr<IMF2DBuffer2> image;
        TX_CHECK(buffer.As(&image));
        BYTE* scanline = nullptr; BYTE* allocation = nullptr; LONG pitch = 0; DWORD length = 0;
        TX_CHECK(image->Lock2DSize(MF2DBuffer_LockFlags_Write, &scanline, &pitch, &allocation, &length));
        last_time_ = (std::max)(MFGetSystemTime(), last_time_ + 1);
        bool locked = true;
        const HRESULT result = mailbox_->write(scanline, pitch, allocation, length, [&]() -> HRESULT {
            const HRESULT unlocked = image->Unlock2D();
            locked = false;
            TX_CHECK(unlocked);
            TX_CHECK(sample->SetSampleTime(last_time_));
            TX_CHECK(sample->SetSampleDuration(FrameDuration));
            if (token) TX_CHECK(sample->SetUnknown(MFSampleExtension_Token, token));
            return events_->QueueEventParamUnk(MEMediaSample, GUID_NULL, S_OK, sample.Get());
        });
        if (locked) image->Unlock2D();
        return result;
    }
    STDMETHODIMP SetStreamState(MF_STREAM_STATE state) override {
        if (state == MF_STREAM_STATE_STOPPED) return stop(false);
        if (state == MF_STREAM_STATE_RUNNING) {
            ComPtr<IMFMediaType> type;
            { std::lock_guard guard(lock_); if (shutdown_) return MF_E_SHUTDOWN; type = type_; }
            return start(type.Get(), false);
        }
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        if (state != MF_STREAM_STATE_PAUSED || state_ != MF_STREAM_STATE_RUNNING) return MF_E_INVALID_STATE_TRANSITION;
        state_ = state; mailbox_->stop(); return S_OK;
    }
    STDMETHODIMP GetStreamState(MF_STREAM_STATE* state) override {
        if (!state) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        *state = state_; return S_OK;
    }
};

class Source final : public RuntimeClass<RuntimeClassFlags<ClassicCom>,
    ChainInterfaces<IMFMediaSourceEx, IMFMediaSource, IMFMediaEventGenerator>,
    IMFGetService, IKsControl, IMFSampleAllocatorControl, IFrameReceiver, IFrameTransport>, public Tracked {
    std::mutex lock_;
    bool shutdown_ = false;
    bool started_ = false;
    bool announced_ = false;
    ComPtr<IMFMediaEventQueue> events_;
    ComPtr<IMFAttributes> attributes_;
    ComPtr<IMFPresentationDescriptor> descriptor_;
    ComPtr<Stream> stream_;
    std::shared_ptr<FrameMailbox> mailbox_ = std::make_shared<FrameMailbox>();
    PipeReceiver transport_{mailbox_};
    VideoPipeEndpoint pending_endpoint_{};
    bool has_pending_endpoint_ = false;
    void clear_pending_endpoint() noexcept {
        has_pending_endpoint_ = false;
        SecureZeroMemory(&pending_endpoint_, sizeof(pending_endpoint_));
    }
    HRESULT event_queue(IMFMediaEventQueue** queue) {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : events_.CopyTo(queue);
    }
public:
    HRESULT initialize(IMFAttributes* activation) {
        TX_CHECK(MFCreateEventQueue(&events_));
        TX_CHECK(MFCreateAttributes(&attributes_, 8));
        if (activation) TX_CHECK(activation->CopyAllItems(attributes_.Get()));
        // Do not expose the ephemeral channel capability through source attributes.
        TX_CHECK(attributes_->DeleteItem(PipeEndpointAttribute));
        ComPtr<IMFSensorProfileCollection> collection;
        ComPtr<IMFSensorProfile> profile;
        TX_CHECK(MFCreateSensorProfileCollection(&collection));
        TX_CHECK(MFCreateSensorProfile(KSCAMERAPROFILE_Legacy, 0, nullptr, &profile));
        TX_CHECK(profile->AddProfileFilter(0, L"((RES==;FRT<=30,1;SUT==))"));
        TX_CHECK(collection->AddProfile(profile.Get()));
        TX_CHECK(attributes_->SetUnknown(MF_DEVICEMFT_SENSORPROFILE_COLLECTION, collection.Get()));
        stream_ = Make<Stream>();
        if (!stream_) return E_OUTOFMEMORY;
        const HRESULT initialized = stream_->initialize(this, mailbox_);
        if (FAILED(initialized)) { stream_->shutdown(); stream_.Reset(); return initialized; }
        ComPtr<IMFStreamDescriptor> stream_descriptor;
        TX_CHECK(stream_->GetStreamDescriptor(&stream_descriptor));
        IMFStreamDescriptor* descriptors[] = {stream_descriptor.Get()};
        TX_CHECK(MFCreatePresentationDescriptor(1, descriptors, &descriptor_));
        TX_CHECK(descriptor_->SelectStream(0));
        if (activation) {
            UINT32 bytes = 0;
            const HRESULT configured = activation->GetBlobSize(PipeEndpointAttribute, &bytes);
            if (configured != MF_E_ATTRIBUTENOTFOUND) {
                TX_CHECK(configured);
                if (bytes != sizeof(VideoPipeEndpoint)) return E_INVALIDARG;
                VideoPipeEndpoint endpoint{};
                TX_CHECK(activation->GetBlob(PipeEndpointAttribute, reinterpret_cast<BYTE*>(&endpoint), sizeof(endpoint), &bytes));
                if (!valid_video_endpoint(endpoint)) { SecureZeroMemory(&endpoint, sizeof(endpoint)); return E_INVALIDARG; }
                // FrameServer creates temporary sources to inspect formats. Such
                // probes must not consume the single-use producer connection.
                pending_endpoint_ = endpoint;
                has_pending_endpoint_ = true;
                SecureZeroMemory(&endpoint, sizeof(endpoint));
            }
        }
        return S_OK;
    }
    TX_EVENTS
    STDMETHODIMP GetCharacteristics(DWORD* characteristics) override {
        if (!characteristics) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        *characteristics = MFMEDIASOURCE_IS_LIVE; return S_OK;
    }
    STDMETHODIMP CreatePresentationDescriptor(IMFPresentationDescriptor** descriptor) override {
        if (!descriptor) return E_POINTER;
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : descriptor_->Clone(descriptor);
    }
    STDMETHODIMP Start(IMFPresentationDescriptor* descriptor, const GUID* format, const PROPVARIANT* position) override {
        if (!descriptor || !position) return E_POINTER;
        if (format && *format != GUID_NULL) return MF_E_UNSUPPORTED_TIME_FORMAT;
        if (position->vt != VT_EMPTY && !(position->vt == VT_I8 && position->hVal.QuadPart == 0)) return MF_E_INVALIDREQUEST;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        if (started_) return MF_E_INVALID_STATE_TRANSITION;
        DWORD count = 0, id = 0; BOOL selected = FALSE;
        TX_CHECK(descriptor->GetStreamDescriptorCount(&count));
        if (count != 1) return E_INVALIDARG;
        ComPtr<IMFStreamDescriptor> sd;
        TX_CHECK(descriptor->GetStreamDescriptorByIndex(0, &selected, &sd));
        TX_CHECK(sd->GetStreamIdentifier(&id));
        if (!selected || id != 0) return E_INVALIDARG;
        ComPtr<IMFMediaTypeHandler> handler;
        ComPtr<IMFMediaType> type;
        TX_CHECK(sd->GetMediaTypeHandler(&handler));
        TX_CHECK(handler->GetCurrentMediaType(&type));
        // Initialize the allocator before advertising a stream that might fail.
        TX_CHECK(stream_->start(type.Get(), false));
        if (has_pending_endpoint_) {
            // One attempt only, when the source actually starts streaming. A
            // failure stays neutral/queryable; no automatic reconnect or fallback.
            transport_.connect(pending_endpoint_);
            clear_pending_endpoint();
        }
        TX_CHECK(events_->QueueEventParamUnk(announced_ ? MEUpdatedStream : MENewStream, GUID_NULL, S_OK, stream_.Get()));
        TX_CHECK(stream_->QueueEvent(MEStreamStarted, GUID_NULL, S_OK, nullptr));
        PROPVARIANT start{}; start.vt = VT_I8; start.hVal.QuadPart = MFGetSystemTime();
        TX_CHECK(events_->QueueEventParamVar(MESourceStarted, GUID_NULL, S_OK, &start));
        announced_ = true; started_ = true;
        return S_OK;
    }
    STDMETHODIMP Stop() override {
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        if (!started_) return MF_E_INVALID_STATE_TRANSITION;
        clear_pending_endpoint();
        transport_.disconnect();
        TX_CHECK(stream_->stop());
        started_ = false;
        return events_->QueueEventParamVar(MESourceStopped, GUID_NULL, S_OK, nullptr);
    }
    STDMETHODIMP Pause() override { return MF_E_INVALID_STATE_TRANSITION; }
    STDMETHODIMP Shutdown() override {
        std::lock_guard guard(lock_);
        if (shutdown_) return S_OK;
        shutdown_ = true; started_ = false;
        clear_pending_endpoint();
        transport_.disconnect();
        mailbox_->stop();
        if (stream_) stream_->shutdown();
        if (events_) events_->Shutdown();
        stream_.Reset(); events_.Reset(); attributes_.Reset(); descriptor_.Reset();
        return S_OK;
    }
    STDMETHODIMP GetSourceAttributes(IMFAttributes** attributes) override {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : attributes_.CopyTo(attributes);
    }
    STDMETHODIMP GetStreamAttributes(DWORD id, IMFAttributes** attributes) override {
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        return id == 0 ? stream_->attributes(attributes) : MF_E_INVALIDSTREAMNUMBER;
    }
    STDMETHODIMP SetD3DManager(IUnknown*) override { return E_NOTIMPL; }
    STDMETHODIMP GetService(REFGUID, REFIID, LPVOID* object) override {
        if (!object) return E_POINTER;
        *object = nullptr; return MF_E_UNSUPPORTED_SERVICE;
    }
    STDMETHODIMP KsProperty(PKSPROPERTY, ULONG, LPVOID, ULONG, ULONG* returned) override {
        if (!returned) return E_POINTER; *returned = 0; return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
    }
    STDMETHODIMP KsMethod(PKSMETHOD, ULONG, LPVOID, ULONG, ULONG* returned) override {
        if (!returned) return E_POINTER; *returned = 0; return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
    }
    STDMETHODIMP KsEvent(PKSEVENT, ULONG, LPVOID, ULONG, ULONG* returned) override {
        if (returned) *returned = 0; return HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND);
    }
    STDMETHODIMP SetDefaultAllocator(DWORD id, IUnknown* allocator) override {
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        return id == 0 ? stream_->set_allocator(allocator) : MF_E_INVALIDSTREAMNUMBER;
    }
    STDMETHODIMP GetAllocatorUsage(DWORD id, DWORD* input, MFSampleAllocatorUsage* usage) override {
        if (!input || !usage) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        if (id != 0) return MF_E_INVALIDSTREAMNUMBER;
        *input = 0; *usage = MFSampleAllocatorUsage_UsesProvidedAllocator; return S_OK;
    }
    STDMETHODIMP BeginVideo(UINT64* generation) override {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : mailbox_->begin(generation);
    }
    STDMETHODIMP SubmitNv12(UINT64 generation, UINT64 id, UINT64 captured, const BYTE* data, UINT32 size) override {
        std::lock_guard guard(lock_);
        return shutdown_ ? MF_E_SHUTDOWN : mailbox_->submit(generation, id, captured, data, size);
    }
    STDMETHODIMP StopVideo() override {
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        clear_pending_endpoint();
        transport_.disconnect();
        mailbox_->stop(); return S_OK;
    }
    STDMETHODIMP GetFrameMetrics(FrameMetrics* metrics) override {
        if (!metrics) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        *metrics = mailbox_->metrics(); return S_OK;
    }
    STDMETHODIMP ConnectPipe(const VideoPipeEndpoint* endpoint) override {
        if (!endpoint) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        clear_pending_endpoint();
        return transport_.connect(*endpoint);
    }
    STDMETHODIMP DisconnectPipe() override {
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        clear_pending_endpoint();
        transport_.disconnect(); return S_OK;
    }
    STDMETHODIMP GetTransportStatus(TransportStatus* status) override {
        if (!status) return E_POINTER;
        std::lock_guard guard(lock_);
        if (shutdown_) return MF_E_SHUTDOWN;
        *status = transport_.status(); return S_OK;
    }
};

class Activation final : public RuntimeClass<RuntimeClassFlags<ClassicCom>,
    ChainInterfaces<IMFActivate, IMFAttributes>>, public Tracked {
    ComPtr<IMFAttributes> attributes_;
    ComPtr<Source> source_;
    std::mutex lock_;
public:
    // The caller of ActivateObject owns the returned source's shutdown. Releasing
    // this activation must not invalidate a source still held by FrameServer.
    // Explicit ShutdownObject below retains its shutdown/release contract.
    HRESULT initialize() { return MFCreateAttributes(&attributes_, 8); }
    STDMETHODIMP ActivateObject(REFIID iid, void** output) override {
        if (!output) return E_POINTER;
        *output = nullptr;
        std::lock_guard guard(lock_);
        try {
            if (!source_) {
                auto source = Make<Source>();
                if (!source) return E_OUTOFMEMORY;
                const HRESULT hr = source->initialize(attributes_.Get());
                if (FAILED(hr)) { source->Shutdown(); return hr; }
                source_ = source;
            }
            return source_->QueryInterface(iid, output);
        } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
    }
    STDMETHODIMP ShutdownObject() override {
        std::lock_guard guard(lock_);
        if (source_) source_->Shutdown(); source_.Reset(); return S_OK;
    }
    STDMETHODIMP DetachObject() override { std::lock_guard guard(lock_); source_.Reset(); return S_OK; }

#define TX_ATTRIBUTE(name, signature, arguments) STDMETHODIMP name signature override { return attributes_->name arguments; }
    TX_ATTRIBUTE(GetItem, (REFGUID key, PROPVARIANT* value), (key, value))
    TX_ATTRIBUTE(GetItemType, (REFGUID key, MF_ATTRIBUTE_TYPE* value), (key, value))
    TX_ATTRIBUTE(CompareItem, (REFGUID key, REFPROPVARIANT value, BOOL* equal), (key, value, equal))
    TX_ATTRIBUTE(Compare, (IMFAttributes* other, MF_ATTRIBUTES_MATCH_TYPE mode, BOOL* equal), (other, mode, equal))
    TX_ATTRIBUTE(GetUINT32, (REFGUID key, UINT32* value), (key, value))
    TX_ATTRIBUTE(GetUINT64, (REFGUID key, UINT64* value), (key, value))
    TX_ATTRIBUTE(GetDouble, (REFGUID key, double* value), (key, value))
    TX_ATTRIBUTE(GetGUID, (REFGUID key, GUID* value), (key, value))
    TX_ATTRIBUTE(GetStringLength, (REFGUID key, UINT32* size), (key, size))
    TX_ATTRIBUTE(GetString, (REFGUID key, LPWSTR value, UINT32 size, UINT32* written), (key, value, size, written))
    TX_ATTRIBUTE(GetAllocatedString, (REFGUID key, LPWSTR* value, UINT32* size), (key, value, size))
    TX_ATTRIBUTE(GetBlobSize, (REFGUID key, UINT32* size), (key, size))
    TX_ATTRIBUTE(GetBlob, (REFGUID key, UINT8* value, UINT32 size, UINT32* written), (key, value, size, written))
    TX_ATTRIBUTE(GetAllocatedBlob, (REFGUID key, UINT8** value, UINT32* size), (key, value, size))
    TX_ATTRIBUTE(GetUnknown, (REFGUID key, REFIID iid, LPVOID* value), (key, iid, value))
    TX_ATTRIBUTE(SetItem, (REFGUID key, REFPROPVARIANT value), (key, value))
    TX_ATTRIBUTE(DeleteItem, (REFGUID key), (key))
    TX_ATTRIBUTE(DeleteAllItems, (), ())
    TX_ATTRIBUTE(SetUINT32, (REFGUID key, UINT32 value), (key, value))
    TX_ATTRIBUTE(SetUINT64, (REFGUID key, UINT64 value), (key, value))
    TX_ATTRIBUTE(SetDouble, (REFGUID key, double value), (key, value))
    TX_ATTRIBUTE(SetGUID, (REFGUID key, REFGUID value), (key, value))
    TX_ATTRIBUTE(SetString, (REFGUID key, LPCWSTR value), (key, value))
    TX_ATTRIBUTE(SetBlob, (REFGUID key, const UINT8* value, UINT32 size), (key, value, size))
    TX_ATTRIBUTE(SetUnknown, (REFGUID key, IUnknown* value), (key, value))
    TX_ATTRIBUTE(LockStore, (), ())
    TX_ATTRIBUTE(UnlockStore, (), ())
    TX_ATTRIBUTE(GetCount, (UINT32* count), (count))
    TX_ATTRIBUTE(GetItemByIndex, (UINT32 index, GUID* key, PROPVARIANT* value), (index, key, value))
    TX_ATTRIBUTE(CopyAllItems, (IMFAttributes* target), (target))
#undef TX_ATTRIBUTE
};

class Factory final : public RuntimeClass<RuntimeClassFlags<ClassicCom>, IClassFactory>, public Tracked {
public:
    STDMETHODIMP CreateInstance(IUnknown* outer, REFIID iid, void** output) override {
        if (!output) return E_POINTER;
        *output = nullptr;
        if (outer) return CLASS_E_NOAGGREGATION;
        try {
            auto activation = Make<Activation>();
            if (!activation) return E_OUTOFMEMORY;
            TX_CHECK(activation->initialize());
            return activation->QueryInterface(iid, output);
        } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
    }
    STDMETHODIMP LockServer(BOOL lock) override {
        if (lock) ++server_locks;
        else {
            long current = server_locks.load();
            do { if (current <= 0) return E_UNEXPECTED; }
            while (!server_locks.compare_exchange_weak(current, current - 1));
        }
        return S_OK;
    }
};
}

extern "C" HRESULT __stdcall DllGetClassObject(REFCLSID clsid, REFIID iid, void** output) {
    if (!output) return E_POINTER;
    *output = nullptr;
    if (clsid != SourceClass) return CLASS_E_CLASSNOTAVAILABLE;
    auto factory = Make<Factory>();
    return factory ? factory->QueryInterface(iid, output) : E_OUTOFMEMORY;
}
extern "C" HRESULT __stdcall DllCanUnloadNow() {
    return objects.load() == 0 && server_locks.load() == 0 ? S_OK : S_FALSE;
}
