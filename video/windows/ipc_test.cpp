// Two real Windows processes, synthetic pixels, no camera registration/capture.
#include "frame_transport.h"
#include "eye_output.h"
#include "../eye/session.h"
#include <mfapi.h>
#include <mfidl.h>
#include <mferror.h>
#include <wrl.h>
#include <aclapi.h>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

using Microsoft::WRL::ComPtr;
using namespace tingxian::video;
namespace tingxian::video {
struct TransportTestPeer {
    static bool restricted_acl(FramePublisher& publisher) {
        PACL acl = nullptr; PSECURITY_DESCRIPTOR descriptor = nullptr;
        if (GetSecurityInfo(publisher.pipe_, SE_KERNEL_OBJECT, DACL_SECURITY_INFORMATION,
            nullptr, nullptr, &acl, nullptr, &descriptor) != ERROR_SUCCESS) return false;
        bool owner = false, service = false, valid = acl && acl->AceCount == 2;
        if (valid) for (DWORD i = 0; i < acl->AceCount; ++i) {
            void* raw = nullptr;
            if (!GetAce(acl, i, &raw)) { valid = false; break; }
            auto ace = static_cast<ACCESS_ALLOWED_ACE*>(raw);
            if (ace->Header.AceType != ACCESS_ALLOWED_ACE_TYPE) { valid = false; break; }
            PSID sid = &ace->SidStart;
            if (IsWellKnownSid(sid, WinLocalServiceSid)) {
                service = (ace->Mask & FILE_WRITE_DATA) && !(ace->Mask & FILE_CREATE_PIPE_INSTANCE);
            } else if (*GetSidSubAuthorityCount(sid) == 3 && *GetSidSubAuthority(sid, 0) == SECURITY_LOGON_IDS_RID) owner = true;
            else valid = false;
        }
        LocalFree(descriptor); return valid && owner && service;
    }
    static HRESULT oversized(FramePublisher& publisher) {
        // Independent malformed wire header; production submit() rejects this size.
        struct Wire { UINT32 magic; UINT16 version, kind; UINT32 bytes, reserved; UINT64 epoch, id, captured; };
        static_assert(sizeof(Wire) == 40);
        const bool timed = publisher.endpoint_.version == 2;
        Wire header{0x31565854, UINT16(publisher.endpoint_.version), UINT16(timed ? 6 : 4),
            0xffffffff, 0, 1, 1, timed ? 0 : GetTickCount64()};
        HANDLE event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (!event) return HRESULT_FROM_WIN32(GetLastError());
        OVERLAPPED operation{}; operation.hEvent = event; DWORD sent = 0;
        BOOL ok = WriteFile(publisher.pipe_, &header, sizeof(header), &sent, &operation);
        HRESULT result = S_OK;
        if (!ok) {
            if (GetLastError() != ERROR_IO_PENDING) result = HRESULT_FROM_WIN32(GetLastError());
            else if (WaitForSingleObject(event, 1000) != WAIT_OBJECT_0) {
                CancelIoEx(publisher.pipe_, &operation); GetOverlappedResult(publisher.pipe_, &operation, &sent, TRUE);
                result = HRESULT_FROM_WIN32(ERROR_TIMEOUT);
            } else if (!GetOverlappedResult(publisher.pipe_, &operation, &sent, FALSE)) result = HRESULT_FROM_WIN32(GetLastError());
        }
        CloseHandle(event); return result;
    }
};
struct EyeOutputTestPeer {
    static bool restricted_acl(EyePipeOutput& output) {
        return TransportTestPeer::restricted_acl(output.publisher_);
    }
};
}
namespace {
int groups = 0;
void expect(bool ok, const char* reason) { if (!ok) throw std::runtime_error(reason); }
void check(HRESULT hr, const char* reason) {
    if (FAILED(hr)) { std::cerr << reason << " HRESULT=0x" << std::hex << unsigned(hr) << std::dec << '\n'; throw std::runtime_error(reason); }
}
void passed(const char* name) { ++groups; std::cout << "PASS " << name << '\n'; }
struct Handle {
    HANDLE value = nullptr;
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
    void reset() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); value = nullptr; }
};
struct OwnedTestProcess {
    HANDLE value = nullptr;
    ~OwnedTestProcess() {
        if (!value) return;
        // This guard also runs if Child's constructor throws before ~Child can.
        if (WaitForSingleObject(value, 0) == WAIT_TIMEOUT) {
            TerminateProcess(value, 4); WaitForSingleObject(value, 1000);
        }
        CloseHandle(value);
    }
};
void pipe_write(HANDLE pipe, const void* data, DWORD size) {
    DWORD written = 0; expect(WriteFile(pipe, data, size, &written, nullptr) && written == size, "control write failed");
}
void pipe_read(HANDLE pipe, void* data, DWORD size) {
    const auto deadline = GetTickCount64() + 4000;
    DWORD available = 0;
    while (GetTickCount64() < deadline) {
        expect(PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr), "control peer disconnected");
        if (available >= size) {
            DWORD got = 0; expect(ReadFile(pipe, data, size, &got, nullptr) && got == size, "control read failed"); return;
        }
        Sleep(1);
    }
    throw std::runtime_error("control peer timeout");
}
struct Command { UINT32 kind; UINT32 reserved = 0; UINT64 epoch = 1; UINT64 id = 1; BYTE y = 160; BYTE uv = 96; BYTE padding[6]{}; TimedFrame timing; };
enum : UINT32 { StartEpoch = 1, Pixels = 2, StopEpoch = 3, Close = 4, Crash = 5, Oversized = 6, ScheduledPixels = 7, QueueProbe = 8 };
struct Ready { HRESULT result = E_FAIL; BOOL restricted_acl = FALSE; VideoPipeEndpoint endpoint{}; };

int publisher_child(HANDLE control, HANDLE report, bool timed = false) {
    try {
        FramePublisher publisher;
        Ready ready;
        ready.result = timed ? publisher.open_timed(&ready.endpoint) : publisher.open(&ready.endpoint);
        if (SUCCEEDED(ready.result)) ready.restricted_acl = TransportTestPeer::restricted_acl(publisher);
        pipe_write(report, &ready, sizeof(ready));
        if (FAILED(ready.result)) return 2;
        HRESULT accepted = publisher.accept(); pipe_write(report, &accepted, sizeof(accepted));
        if (FAILED(accepted)) return 0; // parent checks expected authentication rejection
        std::vector<BYTE> pixels(FrameBytes);
        for (;;) {
            Command command{}; pipe_read(control, &command, sizeof(command));
            HRESULT result = E_INVALIDARG;
            if (command.kind == StartEpoch) result = publisher.begin(command.epoch);
            else if (command.kind == Pixels) {
                std::fill(pixels.begin(), pixels.begin() + Width * Height, command.y);
                std::fill(pixels.begin() + Width * Height, pixels.end(), command.uv);
                result = publisher.submit(command.epoch, command.id, GetTickCount64(), pixels.data(), FrameBytes);
            } else if (command.kind == ScheduledPixels) {
                std::fill_n(pixels.begin(), Width * Height, command.y);
                std::fill(pixels.begin() + Width * Height, pixels.end(), command.uv);
                const bool neutral = command.timing.flags == TimedNeutral;
                result = publisher.submit_timed(command.epoch, command.id, command.timing,
                    neutral ? nullptr : pixels.data(), neutral ? 0 : FrameBytes);
            } else if (command.kind == QueueProbe) {
                // Batch in the producer: parent/child control polling is not the
                // media cadence and must not expire slots before testing capacity.
                UINT64 now=0; if(!qpc_now(now)) return 3;
                for(UINT64 i=1;i<=5;++i) {
                    TimedFrame timing{0,0,now+950000+i*10000,TimedNeutral};
                    result=publisher.submit_timed(command.epoch,i,timing,nullptr,0);
                    if(FAILED(result)) break;
                }
            } else if (command.kind == StopEpoch) result = publisher.end(command.epoch);
            else if (command.kind == Oversized) result = TransportTestPeer::oversized(publisher);
            else if (command.kind == Crash) ExitProcess(0); // deliberately skip destructors in this synthetic child only
            else if (command.kind == Close) { publisher.close(); result = S_OK; }
            pipe_write(report, &result, sizeof(result));
            if (command.kind == Close) return 0;
        }
    } catch (...) { return 3; }
}

// The input is blue and the fake provider returns red. Red MF samples test the
// actual Session -> adapter -> pipe -> source, never an SDK quality claim.
int eye_publisher_child(HANDLE control, HANDLE report) {
    using namespace tingxian::eye;
    struct SyntheticProvider : EyeCorrectionProvider {
        bool fail = false;
        Capabilities capabilities() const noexcept override { return {"synthetic", "1", true}; }
        Error prepare(Format f) override { return f.width==Width && f.height==Height ? Error::none : Error::invalid_frame; }
        Result process(const Frame& f) override {
            if (fail) return {Error::no_face,{}};
            std::vector<BYTE> red(f.bgr.size());
            for(size_t i=2;i<red.size();i+=3) red[i]=255;
            return {Error::none,std::move(red)};
        }
        bool stop() noexcept override { return true; }
    } provider;
    struct TickClock : Clock { uint64_t now_ms() const noexcept override { return GetTickCount64(); } } clock;
    EyePipeOutput output;
    Session session(provider,output,clock);
    struct Cleanup { Session& session; EyePipeOutput& output;
        ~Cleanup(){session.stop();session.drain_stop();output.close();} } cleanup{session,output};
    try {
        const auto generation=session.begin({Width,Height});
        expect(generation && session.prepare(generation),"prepare synthetic eye session");
        Ready ready;
        ready.result=output.open_if_current(generation,output.operation_ticket(),&ready.endpoint);
        if(SUCCEEDED(ready.result))ready.restricted_acl=EyeOutputTestPeer::restricted_acl(output);
        pipe_write(report,&ready,sizeof(ready));
        if(FAILED(ready.result))return 2;
        const auto until=GetTickCount64()+3000;
        while(output.status().state==EyeOutputState::waiting && GetTickCount64()<until)Sleep(1);
        HRESULT handshake=output.status().state==EyeOutputState::streaming ? S_OK : E_FAIL;
        pipe_write(report,&handshake,sizeof(handshake));
        if(FAILED(handshake))return 3;
        for(;;){
            Command command{};pipe_read(control,&command,sizeof(command));
            HRESULT hr=E_INVALIDARG;
            if(command.kind==Pixels){
                provider.fail=command.y==0;
                Frame frame{generation,command.id,GetTickCount64(),std::vector<BYTE>(size_t(Width)*Height*3)};
                for(size_t i=0;i<frame.bgr.size();i+=3)frame.bgr[i]=255;
                hr=session.submit(std::move(frame)) && session.process_one() ? S_OK : E_FAIL;
            }else if(command.kind==StopEpoch || command.kind==Close){
                session.stop();session.drain_stop();output.close();hr=S_OK;
            }else if(command.kind==Crash)ExitProcess(0);
            pipe_write(report,&hr,sizeof(hr));
            if(command.kind==Close)return 0;
        }
    }catch(...){return 3;}
}

struct Child {
    Handle control, report;
    OwnedTestProcess process;
    Ready ready;
    DWORD pid = 0;
    HRESULT handshake = E_PENDING;
    bool crashed = false;
    explicit Child(bool simulate_start_failure = false, HANDLE* observed_process = nullptr, bool eye_output = false, bool timed = false) {
        SECURITY_ATTRIBUTES security{sizeof(security), nullptr, TRUE};
        Handle child_control, child_report;
        expect(CreatePipe(&child_control.value, &control.value, &security, 0), "create control pipe");
        expect(CreatePipe(&report.value, &child_report.value, &security, 0), "create report pipe");
        expect(SetHandleInformation(control.value, HANDLE_FLAG_INHERIT, 0), "private parent control");
        expect(SetHandleInformation(report.value, HANDLE_FLAG_INHERIT, 0), "private parent report");
        SIZE_T attribute_size = 0;
        InitializeProcThreadAttributeList(nullptr, 1, 0, &attribute_size);
        std::vector<BYTE> storage(attribute_size);
        auto attributes = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());
        expect(InitializeProcThreadAttributeList(attributes, 1, 0, &attribute_size), "initialize handle whitelist");
        HANDLE inherited[] = {child_control.value, child_report.value};
        BOOL updated = UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            inherited, sizeof(inherited), nullptr, nullptr);
        if (!updated) { DeleteProcThreadAttributeList(attributes); throw std::runtime_error("handle whitelist failed"); }
        wchar_t executable[32768];
        const DWORD length = GetModuleFileNameW(nullptr, executable, _countof(executable));
        if (!length || length == _countof(executable)) { DeleteProcThreadAttributeList(attributes); throw std::runtime_error("executable path unavailable"); }
        std::wstring command = L"\"" + std::wstring(executable)
            + (eye_output ? L"\" --eye-publisher-test " : timed ? L"\" --timed-publisher-test " : L"\" --publisher-test ")
            + std::to_wstring(reinterpret_cast<UINT_PTR>(child_control.value)) + L" "
            + std::to_wstring(reinterpret_cast<UINT_PTR>(child_report.value));
        STARTUPINFOEXW startup{}; startup.StartupInfo.cb = sizeof(startup); startup.lpAttributeList = attributes;
        PROCESS_INFORMATION created{};
        BOOL ok = CreateProcessW(executable, command.data(), nullptr, nullptr, TRUE,
            CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT, nullptr, nullptr, &startup.StartupInfo, &created);
        DeleteProcThreadAttributeList(attributes);
        expect(ok, "create synthetic producer child");
        process.value = created.hProcess; pid = created.dwProcessId; CloseHandle(created.hThread);
        if (observed_process) expect(DuplicateHandle(GetCurrentProcess(), process.value, GetCurrentProcess(), observed_process,
            SYNCHRONIZE, FALSE, 0), "duplicate test process observation handle");
        if (simulate_start_failure) throw std::runtime_error("synthetic child startup failure");
        child_control.reset(); child_report.reset();
        pipe_read(report.value, &ready, sizeof(ready)); check(ready.result, "publisher create");
        expect(ready.endpoint.producer_pid == pid, "publisher PID differs from created child");
        expect(ready.restricted_acl, "live pipe ACL is broader than allowed");
    }
    ~Child() {
        if (!crashed && handshake == S_OK && WaitForSingleObject(process.value, 0) == WAIT_TIMEOUT) {
            try { send({Close}); } catch (...) { /* Preserve the original test failure; the exact-child guard remains. */ }
        }
        control.reset(); report.reset();
        if (process.value) WaitForSingleObject(process.value, 1000);
    }
    HRESULT accepted() { pipe_read(report.value, &handshake, sizeof(handshake)); return handshake; }
    HRESULT send(Command command) {
        pipe_write(control.value, &command, sizeof(command));
        if (command.kind == Crash) { crashed = true; return S_OK; }
        HRESULT result; pipe_read(report.value, &result, sizeof(result)); return result;
    }
};

ComPtr<IMFMediaEvent> next_event(IMFMediaEventGenerator* generator, MediaEventType wanted) {
    const auto end = GetTickCount64() + 2000;
    while (GetTickCount64() < end) {
        ComPtr<IMFMediaEvent> event;
        HRESULT hr = generator->GetEvent(MF_EVENT_FLAG_NO_WAIT, &event);
        if (hr == MF_E_NO_EVENTS_AVAILABLE) { Sleep(1); continue; }
        check(hr, "MF event"); MediaEventType type; check(event->GetType(&type), "MF event type");
        if (type == wanted) return event;
    }
    throw std::runtime_error("MF event timeout");
}
struct SourceFixture {
    HMODULE library = nullptr;
    ComPtr<IMFActivate> activation;
    ComPtr<IMFMediaSource> source;
    ComPtr<IMFMediaStream> stream;
    ComPtr<IFrameReceiver> receiver;
    ComPtr<IFrameTransport> transport;
    explicit SourceFixture(const wchar_t* path, const VideoPipeEndpoint* endpoint = nullptr, bool start_stream = true) {
        library = LoadLibraryExW(path, nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
        expect(library, "load media source");
        auto get_class = reinterpret_cast<HRESULT(__stdcall*)(REFCLSID, REFIID, void**)>(GetProcAddress(library, "DllGetClassObject"));
        expect(get_class, "class factory export missing");
        ComPtr<IClassFactory> factory; check(get_class(SourceClass, IID_PPV_ARGS(&factory)), "factory");
        check(factory->CreateInstance(nullptr, IID_PPV_ARGS(&activation)), "activate factory");
        if (endpoint) check(activation->SetBlob(PipeEndpointAttribute, reinterpret_cast<const BYTE*>(endpoint), sizeof(*endpoint)), "configure transient endpoint");
        check(activation->ActivateObject(IID_PPV_ARGS(&source)), "source");
        check(source.As(&receiver), "receiver"); check(source.As(&transport), "transport");
        if (start_stream) start();
    }
    void start() {
        ComPtr<IMFSampleAllocatorControl> control; check(source.As(&control), "allocator control");
        ComPtr<IMFVideoSampleAllocator> allocator;
        check(MFCreateVideoSampleAllocatorEx(IID_PPV_ARGS(&allocator)), "allocator");
        check(control->SetDefaultAllocator(0, allocator.Get()), "set allocator");
        ComPtr<IMFPresentationDescriptor> pd; check(source->CreatePresentationDescriptor(&pd), "presentation");
        PROPVARIANT start{}; check(source->Start(pd.Get(), nullptr, &start), "MF start");
        auto event = next_event(source.Get(), MENewStream);
        PROPVARIANT value{}; check(event->GetValue(&value), "stream value");
        HRESULT hr = value.vt == VT_UNKNOWN ? value.punkVal->QueryInterface(IID_PPV_ARGS(&stream)) : E_UNEXPECTED;
        PropVariantClear(&value); check(hr, "stream");
        next_event(source.Get(), MESourceStarted); next_event(stream.Get(), MEStreamStarted);
    }
    ~SourceFixture() {
        if (source) source->Shutdown();
        if (activation) activation->ShutdownObject();
        transport.Reset(); receiver.Reset(); stream.Reset(); source.Reset(); activation.Reset();
        if (library) FreeLibrary(library);
    }
    FrameMetrics metrics() { FrameMetrics value; check(receiver->GetFrameMetrics(&value), "metrics"); return value; }
    TransportStatus status() { TransportStatus value; check(transport->GetTransportStatus(&value), "transport status"); return value; }
    template<class Predicate> void wait(Predicate predicate, const char* message) {
        const auto deadline = GetTickCount64() + 2000;
        while (GetTickCount64() < deadline) { if (predicate()) return; Sleep(1); }
        throw std::runtime_error(message);
    }
    void pixels(BYTE y_value, BYTE uv_value, int v_value = -1, TimedSample* timing = nullptr, LONGLONG* pts = nullptr) {
        check(stream->RequestSample(nullptr), "request IPC sample");
        auto event = next_event(stream.Get(), MEMediaSample);
        PROPVARIANT value{}; check(event->GetValue(&value), "sample value");
        ComPtr<IMFSample> sample;
        HRESULT hr = value.vt == VT_UNKNOWN ? value.punkVal->QueryInterface(IID_PPV_ARGS(&sample)) : E_UNEXPECTED;
        PropVariantClear(&value); check(hr, "sample");
        if (timing) { UINT32 size = 0; check(sample->GetBlob(SampleTimingAttribute,
            reinterpret_cast<BYTE*>(timing), sizeof(*timing), &size), "actual MF timing metadata");
            expect(size == sizeof(*timing) && timing->version == 2, "MF timing ABI"); }
        if (pts) check(sample->GetSampleTime(pts), "actual MF sample PTS");
        ComPtr<IMFMediaBuffer> buffer; check(sample->GetBufferByIndex(0, &buffer), "buffer");
        ComPtr<IMF2DBuffer2> image; check(buffer.As(&image), "2D buffer");
        BYTE *scanline = nullptr, *base = nullptr; LONG pitch = 0; DWORD length = 0;
        check(image->Lock2DSize(MF2DBuffer_LockFlags_Read, &scanline, &pitch, &base, &length), "lock pixels");
        bool valid = pitch >= LONG(Width);
        if (valid) for (UINT32 row = 0; row < Height + Height / 2; ++row)
            for (UINT32 col = 0; col < Width; ++col)
                if (scanline[size_t(row) * pitch + col]
                    != BYTE(row < Height ? y_value : ((col % 2 && v_value >= 0) ? v_value : uv_value))) valid = false;
        check(image->Unlock2D(), "unlock pixels"); expect(valid, "cross-process pixels differ");
    }
    void connect(Child& child) {
        check(transport->ConnectPipe(&child.ready.endpoint), "authenticated pipe connect");
        check(child.accepted(), "producer peer authorization");
    }
};

void run(const wchar_t* dll) {
    {
        SourceFixture timed_source(dll);
        ComPtr<ITimedFrameReceiver> timed; check(timed_source.source.As(&timed), "explicit v2 interface");
        UINT32 version=0; check(timed->GetTimingVersion(&version), "timing capability"); expect(version==2, "not v2");
        Child child(false,nullptr,false,true); timed_source.connect(child);
        expect(child.ready.endpoint.version==2, "timing protocol not explicit");
        check(child.send({StartEpoch}), "timed epoch admitted");
        Command cmd{ScheduledPixels}; UINT64 now=0; expect(qpc_now(now),"QPC");
        cmd.timing={41,now,now+800000};
        check(child.send(cmd),"cross-process scheduled frame admitted");
        TimedSample stamp; LONGLONG pts=0;
        timed_source.pixels(32,128,-1,&stamp,&pts);
        expect(stamp.sequence==0 && !stamp.capture_qpc && UINT64(pts)<cmd.timing.presentation_qpc,
            "future image released early or test missed future window");
        const auto deadline=GetTickCount64()+1000;
        do { Sleep(1); expect(qpc_now(now),"QPC wait"); } while(now<cmd.timing.presentation_qpc && GetTickCount64()<deadline);
        expect(now>=cmd.timing.presentation_qpc,"planned time wait expired");
        timed_source.pixels(160,96,-1,&stamp,&pts);
        expect(stamp.epoch==1 && stamp.sequence==1 && stamp.frame_id==41 && stamp.capture_qpc==cmd.timing.capture_qpc &&
            stamp.presentation_qpc==cmd.timing.presentation_qpc && UINT64(pts)==cmd.timing.presentation_qpc && stamp.flags==0,
            "actual MF sample retimed or lost original capture");
        passed("v2 cross-process admission preserves due pixels, capture QPC and exact MF PTS");
        timed_source.pixels(160,96,-1,&stamp,&pts);
        expect(stamp.flags==TimedRepeated && stamp.capture_qpc==cmd.timing.capture_qpc &&
            UINT64(pts)>cmd.timing.presentation_qpc,"repeat refreshed original capture or PTS");
        expect(FAILED(child.send(cmd)),"v2 duplicate silently accepted");
        expect(FAILED(child.send({Pixels})),"v1 payload allowed on v2");
        cmd.id=2; cmd.epoch=9; expect(FAILED(child.send(cmd)),"wrong epoch accepted");
        check(child.send({StopEpoch}),"remote timed stop acknowledged");
        TimedMetrics metrics; check(timed->GetTimedMetrics(&metrics),"timed metrics");
        expect(!metrics.enabled && metrics.queued==0 && metrics.unique==1 && metrics.repeated==1 && metrics.rejected==2,"receipt/count semantics");
        timed_source.pixels(32,128,-1,&stamp,&pts);
        expect(!stamp.frame_id&&!stamp.capture_qpc&&!stamp.sequence,"stop sample retained identity");
        passed("v2 rejection receipts, repeats and stopped MF samples remain explicit");
        for(UINT64 epoch=2;epoch<=4;++epoch){
            Command start{StartEpoch};start.epoch=epoch;check(child.send(start),"v2 restart");
            cmd.epoch=epoch;cmd.id=1;expect(qpc_now(now),"restart QPC");cmd.timing={0,0,now+500000,TimedNeutral};
            check(child.send(cmd),"scheduled neutral admitted");
            Command end{StopEpoch};end.epoch=epoch;check(child.send(end),"queued stop ack");
            check(timed->GetTimedMetrics(&metrics),"stopped queue");expect(metrics.queued==0,"stop ack before queue clear");
        }
        expect(metrics.cleared==3,"pending neutrals not accounted");
        Command start{StartEpoch};start.epoch=5;check(child.send(start),"backpressure epoch");
        Command probe{QueueProbe};probe.epoch=5;
        expect(child.send(probe)==HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW),"wire hid queue backpressure");
        check(timed->GetTimedMetrics(&metrics),"backpressure queue");expect(metrics.queued==4,"wire queue exceeded cap");
        Command end{StopEpoch};end.epoch=5;check(child.send(end),"backpressure cleanup");
        passed("v2 receipt exposes bounded-queue backpressure to its producer");
        check(child.send({Close}),"close timed child");
        timed_source.wait([&]{return timed_source.status().state==TransportState::fault;},"timed disconnect missing");
        timed_source.pixels(32,128); timed.Reset();
        passed("three v2 restarts clear queued plans before stop receipts");
    }
    {
        SourceFixture source(dll); Child child(false,nullptr,false,true);
        auto legacy=child.ready.endpoint;legacy.version=1;
        expect(FAILED(source.transport->ConnectPipe(&legacy)),"v2 silently downgraded");
        expect(FAILED(child.accepted()),"producer accepted downgraded handshake");
        source.pixels(32,128); passed("v2 rejects a v1 handshake without protocol downgrade");
    }
    {
        SourceFixture source(dll); Child child(false,nullptr,false,true);source.connect(child);
        check(child.send({StartEpoch}),"malformed test begin");
        check(child.send({Oversized}),"v2 oversized wire");
        source.wait([&]{return source.status().state==TransportState::fault;},"oversized v2 was not rejected");
        expect(source.status().last_error==E_INVALIDARG,"malformed v2 error hidden");
        source.pixels(32,128); passed("v2 rejects oversized declarations before payload allocation/read");
    }
    {
        Handle observed;
        bool failed = false;
        try { Child child(true, &observed.value); }
        catch (const std::exception&) { failed = true; }
        expect(failed && observed.value && WaitForSingleObject(observed.value, 0) == WAIT_OBJECT_0,
            "child survived a constructor failure");
        passed("startup failure releases only the test's exact child process");
    }
    SourceFixture source(dll);
    source.pixels(32, 128);
    for (int failure=0;failure<3;++failure) {
        Child child(false,nullptr,true);source.connect(child);
        const auto accepted=source.metrics().accepted;
        check(child.send({Pixels}),"processed eye frame");
        source.wait([&]{return source.metrics().accepted>accepted;},"corrected eye frame not received");
        source.pixels(63,102,240); // provider's red, never input's blue
        if(failure==0)check(child.send({StopEpoch}),"stop eye provider");
        else if(failure==1){Command command{Pixels};command.id=2;command.y=0;
            expect(FAILED(child.send(command)),"no-face provider result falsely completed");}
        else check(child.send({Crash}),"crash synthetic eye process");
        source.wait([&]{return source.status().state==TransportState::fault;},"eye stop/fault did not disconnect output");
        source.pixels(32,128);
        expect(!source.metrics().enabled,"eye stop/fault retained enabled output");
        passed(failure==0 ? "corrected provider pixels cross processes into MF; explicit stop neutralizes"
            : failure==1 ? "provider no-face fault neutralizes actual MF output without raw fallback"
            : "eye producer crash neutralizes MF output without worker cleanup");
    }
    {
        Child child; source.connect(child);
        check(child.send({StartEpoch}), "begin epoch");
        check(child.send({Pixels}), "send frame");
        source.wait([&] { return source.metrics().accepted >= 1; }, "frame was not received");
        source.pixels(160, 96);
        passed("two processes authenticate and preserve all NV12 pixels");
        check(child.send({StopEpoch}), "stop epoch");
        source.wait([&] { return source.status().state == TransportState::stopped; }, "stop was not received");
        source.pixels(32, 128);
        Command start{StartEpoch}; start.epoch = 2; check(child.send(start), "begin new epoch");
        const auto rejected = source.metrics().rejected;
        Command late{Pixels}; late.id = 2; check(child.send(late), "send late old epoch");
        source.wait([&] { return source.metrics().rejected > rejected; }, "late epoch not discarded");
        Command fresh{Pixels}; fresh.epoch = 2; fresh.y = 90; fresh.uv = 140;
        const auto accepted = source.metrics().accepted;
        check(child.send(fresh), "send new epoch");
        source.wait([&] { return source.metrics().accepted > accepted; }, "new epoch not accepted");
        source.pixels(90, 140);
        passed("remote stop and explicit new epoch isolate late frames");
        check(child.send({Crash}), "simulate abrupt producer exit");
        source.wait([&] { return source.status().state == TransportState::fault; }, "producer exit not observed");
        source.pixels(32, 128); expect(!source.metrics().enabled, "disconnected producer still enabled");
        passed("OS pipe disconnect neutralizes an abruptly exited producer");
    }
    check(source.transport->DisconnectPipe(), "clear disconnected transport");
    {
        Child child; source.connect(child);
        const auto started = GetTickCount64();
        check(source.transport->DisconnectPipe(), "cancel blocked read");
        expect(GetTickCount64() - started < 500, "blocked read cancellation exceeded 500 ms");
        source.pixels(32, 128);
        passed("explicit disconnect cancels a pending read and retains neutral output");
    }
    for (int mode = 0; mode < 3; ++mode) {
        Child child; auto endpoint = child.ready.endpoint;
        if (mode == 0) endpoint.producer_pid = GetCurrentProcessId();
        if (mode == 1) endpoint.producer_session ^= 0x80000000;
        if (mode == 2) endpoint.capability[0] ^= 0x80;
        expect(FAILED(source.transport->ConnectPipe(&endpoint)), "invalid connection identity accepted");
        expect(FAILED(child.accepted()), "producer accepted invalid handshake");
        source.pixels(32, 128); expect(!source.metrics().enabled, "failed authentication started video");
    }
    passed("wrong producer PID, session and capability each fail closed");
    {
        VideoPipeEndpoint endpoint{};
        wcscpy_s(endpoint.name, L"\\\\server\\pipe\\TingXian.Video.00000000000000000000000000000000");
        endpoint.producer_pid = GetCurrentProcessId(); endpoint.capability[0] = 1;
        expect(source.transport->ConnectPipe(&endpoint) == E_INVALIDARG, "remote path accepted");
        source.pixels(32, 128);
        passed("nonlocal pipe path rejected before opening a connection");
    }
    {
        Child child; source.connect(child);
        check(child.send({StartEpoch}), "begin malformed case");
        check(child.send({Oversized}), "send oversized wire declaration");
        source.wait([&] { return source.status().state == TransportState::fault; }, "oversized header accepted");
        expect(source.status().last_error == E_INVALIDARG, "wrong oversized rejection");
        source.pixels(32, 128);
        passed("untrusted oversized payload rejected before allocation or read");
    }
    {
        Child child; source.connect(child);
        check(child.send({StartEpoch}), "fresh begin after protocol failure");
        const auto accepted = source.metrics().accepted;
        check(child.send({Pixels}), "fresh frame after protocol failure");
        source.wait([&] { return source.metrics().accepted > accepted; }, "explicit reconnect failed");
        source.pixels(160, 96);
        check(source.receiver->StopVideo(), "user stop with active IPC");
        expect(source.status().state == TransportState::disconnected, "user stop left IPC active");
        source.pixels(32, 128);
        passed("explicit reconnect recovers while user stop closes the transport");
    }
    {
        Child child;
        {
            SourceFixture probe(dll, &child.ready.endpoint, false);
            expect(probe.status().state == TransportState::disconnected,
                "format probe consumed producer connection");
            expect(!probe.metrics().enabled && probe.metrics().accepted == 0,
                "format probe enabled frame input");
            DWORD available = 0;
            expect(PeekNamedPipe(child.report.value, nullptr, 0, nullptr, &available, nullptr)
                && available == 0, "format probe completed a producer handshake");
        }
        SourceFixture configured(dll, &child.ready.endpoint);
        check(child.accepted(), "activation-configured handshake");
        ComPtr<IMFMediaSourceEx> extended; check(configured.source.As(&extended), "source attributes interface");
        ComPtr<IMFAttributes> attributes; check(extended->GetSourceAttributes(&attributes), "source attributes");
        UINT32 bytes = 0;
        expect(attributes->GetBlobSize(PipeEndpointAttribute, &bytes) == MF_E_ATTRIBUTENOTFOUND, "capability leaked in source attributes");
        check(child.send({StartEpoch}), "activation begin"); check(child.send({Pixels}), "activation frame");
        configured.wait([&] { return configured.metrics().accepted == 1; }, "activation-configured frame missing");
        configured.pixels(160, 96);
        passed("format probing preserves the producer for actual streaming activation");
        passed("MF activation config connects IPC without exposing its capability");
    }
    {
        Child child;
        SourceFixture stopped(dll, &child.ready.endpoint, false);
        check(stopped.receiver->StopVideo(), "revoke pending activation endpoint");
        stopped.start();
        expect(stopped.status().state == TransportState::disconnected,
            "late MF start reconnected after explicit video stop");
        stopped.pixels(32, 128);
        passed("explicit stop revokes an endpoint before delayed stream start");
    }
}
// Rust's production preview/output adapter drives this reader over stdin/stdout.
// The capability is an inherited pipe payload, never a command-line/log value.
// Actual COM/MF samples, no virtual camera registration or physical capture.
void relay_reader(const wchar_t* dll, bool timed = false) {
    const auto input = GetStdHandle(STD_INPUT_HANDLE), output = GetStdHandle(STD_OUTPUT_HANDLE);
    expect(GetFileType(input) == FILE_TYPE_PIPE && GetFileType(output) == FILE_TYPE_PIPE, "relay test requires private pipes");
    VideoPipeEndpoint endpoint{}; pipe_read(input, &endpoint, sizeof(endpoint));
    expect(valid_video_endpoint(endpoint) && endpoint.version == (timed ? 2u : 1u), "relay endpoint invalid");
    SourceFixture source(dll, &endpoint); SecureZeroMemory(&endpoint, sizeof(endpoint));
    BYTE reply = 'R'; pipe_write(output, &reply, 1); UINT64 expected = 0;
    for (;;) {
        BYTE command = 0; pipe_read(input, &command, 1);
        if (command == 'T' && timed) {
            TimedFrame expected_frame;pipe_read(input,&expected_frame,sizeof(expected_frame));
            source.wait([&]{UINT64 now=0;return qpc_now(now)&&now>=expected_frame.presentation_qpc;},"planned sample not due");
            TimedSample stamp;LONGLONG pts=0;
            const bool neutral=expected_frame.flags==TimedNeutral;
            source.pixels(neutral?32:160,neutral?128:96,-1,&stamp,&pts);
            expect(stamp.frame_id==expected_frame.frame_id&&stamp.capture_qpc==expected_frame.capture_qpc&&
                stamp.presentation_qpc==expected_frame.presentation_qpc&&UINT64(pts)==expected_frame.presentation_qpc,
                "Rust v2 timeline lost actual MF time/identity");
        } else if (command == 'P' && !timed) {
            ++expected; source.wait([&]{return source.metrics().accepted >= expected;}, "relay pixels not received");
            source.pixels(188, 128); // synthetic worker's corrected gray, not input gray
        } else if (command == 'N') {
            source.wait([&]{return source.status().state == TransportState::fault;}, "relay stop not observed");
            source.pixels(32, 128); expect(!source.metrics().enabled, "relay retained old frame");
        } else if (command == 'Q') return;
        else throw std::runtime_error("unknown relay test command");
        pipe_write(output, &command, 1);
    }
}
}

int wmain(int argc, wchar_t** argv) {
    if (argc == 4 && wcscmp(argv[1], L"--timed-publisher-test") == 0)
        return publisher_child(reinterpret_cast<HANDLE>(_wcstoui64(argv[2], nullptr, 10)), reinterpret_cast<HANDLE>(_wcstoui64(argv[3], nullptr, 10)), true);
    if (argc == 4 && wcscmp(argv[1], L"--eye-publisher-test") == 0)
        return eye_publisher_child(reinterpret_cast<HANDLE>(_wcstoui64(argv[2], nullptr, 10)), reinterpret_cast<HANDLE>(_wcstoui64(argv[3], nullptr, 10)));
    if (argc == 4 && wcscmp(argv[1], L"--publisher-test") == 0)
        return publisher_child(reinterpret_cast<HANDLE>(_wcstoui64(argv[2], nullptr, 10)), reinterpret_cast<HANDLE>(_wcstoui64(argv[3], nullptr, 10)));
    const bool timed_relay = argc == 3 && wcscmp(argv[1], L"--timed-relay-reader") == 0;
    const bool relay = timed_relay || (argc == 3 && wcscmp(argv[1], L"--relay-reader") == 0);
    if (argc != 2 && !relay) return 2;
    bool com = false, mf = false; int result = 1;
    try {
        check(CoInitializeEx(nullptr, COINIT_MULTITHREADED), "COM initialize"); com = true;
        check(MFStartup(MF_VERSION, MFSTARTUP_FULL), "MF initialize"); mf = true;
        if (relay) relay_reader(argv[2],timed_relay);
        else {
            run(argv[1]);
            std::cout << "{\"checks\":" << groups << ",\"result\":\"pass\",\"two_processes\":true,\"synthetic_only\":true,\"device_registered\":false}\n";
        }
        result = 0;
    } catch (const std::exception& error) { std::cerr << "FAIL " << error.what() << '\n'; }
    if (mf) MFShutdown(); if (com) CoUninitialize();
    return result;
}
