#include "input_frame.h"
#include "input_camera.h"
#include "video_timing.h"
#include <mfapi.h>
#include <mferror.h>
#include <wrl/client.h>
#include <iostream>
#include <stdexcept>
#include <thread>
using namespace tingxian::camera_input;
using Microsoft::WRL::ComPtr;
namespace {
int assertions = 0, groups = 0;
void expect(bool value, const char* name) { ++assertions; if (!value) throw std::runtime_error(name); }
void ok(HRESULT hr, const char* name) { if (FAILED(hr)) std::cerr << name << " HRESULT=" << std::hex << UINT32(hr) << std::dec << '\n'; expect(SUCCEEDED(hr), name); }
void pass(const char* name) { ++groups; std::cout << "PASS " << name << '\n'; }
ComPtr<IMFSample> sample(UINT64 qpc, bool two = false, bool bottom_up = false) {
    ComPtr<IMFSample> s; ok(MFCreateSample(&s), "sample");
    ComPtr<IMFMediaBuffer> b;
    if (two) ok(MFCreate2DMediaBuffer(Width, Height, MFVideoFormat_RGB32.Data1, bottom_up, &b), "2D buffer");
    else ok(MFCreateMemoryBuffer(Width * Height * 4, &b), "memory buffer");
    // Fill through the same top-row contract the decoder consumes. Distinct
    // corners catch channel order, vertical flip and ignored pitch/padding.
    ComPtr<IMF2DBuffer> plane;
    BYTE* start = nullptr; LONG pitch = LONG(Width * 4); DWORD capacity = 0;
    if (two) { ok(b.As(&plane), "2D QI"); ok(plane->Lock2D(&start, &pitch), "2D lock"); }
    else ok(b->Lock(&start, &capacity, nullptr), "memory lock");
    for (UINT32 y = 0; y < Height; ++y) {
        auto* row = start + static_cast<INT64>(pitch) * y;
        for (UINT32 x = 0; x < Width; ++x) {
            row[x * 4] = BYTE(x % 251); row[x * 4 + 1] = BYTE(y % 253); row[x * 4 + 2] = 217; row[x * 4 + 3] = 91;
        }
    }
    if (two) ok(plane->Unlock2D(), "2D unlock");
    else { ok(b->Unlock(), "memory unlock"); ok(b->SetCurrentLength(Width * Height * 4), "length"); }
    ok(s->AddBuffer(b.Get()), "add buffer");
    if (qpc) ok(s->SetUINT64(MFSampleExtension_DeviceTimestamp, qpc), "device QPC");
    ok(s->SetSampleTime(1234), "different relative sample time");
    return s;
}
void pixels(const Frame& f) {
    expect(f.bgr.size() == BgrBytes, "fixed BGR capacity");
    for (const auto y : {0u, 37u, Height - 1}) for (const auto x : {0u, 17u, Width - 1}) {
        const auto at = (y * Width + x) * 3;
        expect(f.bgr[at] == x % 251 && f.bgr[at + 1] == y % 253 && f.bgr[at + 2] == 217, "BGR oriented pixels");
    }
}
void run() {
    constexpr UINT64 time = 9007199254740993ull; // retain exact integers above JS precision
    for (const auto variant : {0, 1, 2}) {
        auto s = sample(time, variant != 0, variant == 2); Frame frame;
        ok(decode(s.Get(), LONG(Width * 4), frame), "decode exact native sample");
        expect(frame.capture_qpc == time, "not arrival/sample-relative timestamp"); pixels(frame);
    }
    pass("RGB32 memory and 2D positive/negative stride preserve BGR and original QPC");
    {
        std::vector<BYTE> bytes((Width * 4 + 32) * Height, 77); Frame f;
        ok(copy_bgr(bytes.data(), bytes.size(), 0, LONG(Width * 4 + 32), f), "padding allowed");
        expect(f.bgr.front() == 77 && f.bgr.back() == 77, "padded rows copied");
        expect(FAILED(copy_bgr(bytes.data(), bytes.size(), bytes.size(), LONG(Width * 4), f)) && f.bgr.empty(), "offset overflow rejects and clears");
        expect(FAILED(copy_bgr(bytes.data(), bytes.size(), 0, -LONG(Width * 4), f)), "negative first row underflow");
        expect(FAILED(copy_bgr(bytes.data(), bytes.size(), 0, LONG(Width * 4 - 1), f)), "short stride");
        expect(FAILED(copy_bgr(bytes.data(), 8, 0, LONG(Width * 4), f)), "short buffer");
        expect(FAILED(copy_bgr(bytes.data(), bytes.size(), SIZE_MAX, LONG_MAX, f)), "overflow offset");
        auto s = sample(0);
        expect(decode(s.Get(), LONG(Width * 4), f) == MF_E_NO_SAMPLE_TIMESTAMP && f.bgr.empty(), "missing device timestamp never falls back");
        s = sample(UINT64_MAX);
        expect(decode(s.Get(), LONG(Width * 4), f) == MF_E_NO_SAMPLE_TIMESTAMP, "invalid time domain");
        s = sample(time); ComPtr<IMFMediaBuffer> extra; ok(MFCreateMemoryBuffer(4, &extra), "extra buffer"); ok(s->AddBuffer(extra.Get()), "extra sample plane");
        expect(decode(s.Get(), LONG(Width * 4), f) == MF_E_INVALID_STREAM_DATA, "unexpected multi-buffer layout");
    }
    pass("missing time and malformed memory reject without fallback or leftover pixels");
    {
        Mailbox box(7); Frame f; auto s = sample(time);
        expect(box.valid(), "mailbox events");
        box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + 1);
        expect(box.take(6, time + 2, f) == E_INVALIDARG && f.bgr.empty(), "old generation cannot drain new frame");
        ok(box.take(7, time + 2, f), "take original");
        expect(f.generation == 7 && f.sequence == 1 && f.capture_qpc == time, "immutable provenance"); pixels(f);
        expect(box.take(7, time + 3, f) == S_FALSE && f.bgr.empty(), "no repeated frame");
        for (UINT64 i = 1; i <= 20; ++i) {
            ok(s->SetUINT64(MFSampleExtension_DeviceTimestamp, time + i), "advance time");
            box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + 21);
            expect(box.counts().queued == 1, "one-slot hard bound");
        }
        const auto c = box.counts(); expect(c.accepted == 21 && c.superseded == 19 && c.delivered == 1, "exact replacement accounting");
        ok(box.take(7, time + 22, f), "latest frame"); expect(f.sequence == 21 && f.capture_qpc == time + 20, "latest ID and time");
        box.close(); expect(box.take(7, time + 23, f) == MF_E_SHUTDOWN && f.bgr.empty(), "closed output cleared");
        box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + 24);
        expect(box.counts().late == 1 && box.counts().queued == 0, "late callback rejected");
    }
    pass("single-slot generation, supersession, no replay and stopped callback gate");
    {
        Mailbox box(9); Frame f; auto s = sample(time);
        box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + MaxAge + 1);
        expect(box.counts().stale == 1 && !box.counts().queued && !box.counts().closed, "arrival age drops with count");
        ok(s->SetUINT64(MFSampleExtension_DeviceTimestamp, time + MaxAge + 2), "new capture");
        ok(s->SetUINT32(MFSampleExtension_Discontinuity, 1), "discontinuity");
        box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + MaxAge + 3);
        expect(box.counts().discontinuities == 1, "video gap retained");
        expect(box.take(9, time + 2 * MaxAge + 3, f) == S_FALSE && f.bgr.empty(), "expiry rechecked at delivery");
        expect(box.counts().stale == 2, "both expiry locations counted");
    }
    pass("arrival and delivery age gates preserve gaps without accumulating video");
    {
        Mailbox box(8); Frame f; auto s = sample(time);
        box.sample(S_OK, 0, MF_SOURCE_READERF_STREAMTICK, nullptr, LONG(Width * 4), time);
        expect(box.take(8, time, f) == S_FALSE && !box.counts().closed, "stream gap wakes owner without fake frame");
        ok(s->SetUINT32(MFSampleExtension_Discontinuity, 1), "legitimate video gap");
        box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + 1);
        ok(box.take(8, time + 1, f), "gap frame accepted");
        expect(f.discontinuity == 1 && box.counts().discontinuities == 1, "gap flag reaches consumer");
        box.close();
        Mailbox next(9);
        next.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + 2);
        box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + 2);
        ok(next.take(9, time + 2, f), "new lifetime separate slot");
        expect(f.generation == 9 && f.sequence == 1 && box.counts().late == 1, "old callback cannot poison restart");
    }
    pass("video discontinuity remains explicit and old generation cannot affect restart");
    for (int kind = 0; kind < 8; ++kind) {
        Mailbox box(1); Frame f; auto s = sample(time);
        box.sample(S_OK, 0, 0, s.Get(), LONG(Width * 4), time + 1);
        HRESULT error = S_OK; DWORD stream = 0, flags = 0;
        if (kind == 0) ok(s->DeleteItem(MFSampleExtension_DeviceTimestamp), "remove time");
        if (kind == 1) ok(s->SetUINT64(MFSampleExtension_DeviceTimestamp, time + 100), "future time");
        if (kind == 2) ok(s->SetUINT64(MFSampleExtension_DeviceTimestamp, time - 1), "backward time");
        // kind 3 is duplicate timestamp
        if (kind == 4) flags = MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED;
        if (kind == 5) flags = MF_SOURCE_READERF_ENDOFSTREAM;
        if (kind == 6) error = E_ACCESSDENIED;
        if (kind == 7) stream = 1;
        box.sample(error, stream, flags, s.Get(), LONG(Width * 4), time + 2);
        expect(box.counts().closed && FAILED(box.counts().error) && box.counts().queued == 0, "fault revokes queued raw frame");
        expect(FAILED(box.take(1, time + 3, f)) && f.bgr.empty(), "fault cannot expose previous frame");
    }
    pass("device/permission, clock, stream and format failures revoke pending frames");
    {
        auto box = std::make_shared<Mailbox>(42); ComPtr<IMFSourceReaderCallback> callback;
        ok(make_callback(box, LONG(Width * 4), &callback), "production callback");
        UINT64 now = 0; expect(tingxian::video::qpc_now(now), "QPC"); auto s = sample(now);
        ok(callback->OnReadSample(S_OK, 0, 0, 999, s.Get()), "real callback memory sample");
        Frame f; expect(tingxian::video::qpc_now(now), "QPC read"); ok(box->take(42, now, f), "callback original time");
        expect(f.capture_qpc != 999 && f.capture_qpc > 0, "callback relative time ignored"); pixels(f);
        box->close();
        std::thread late([&] { callback->OnReadSample(S_OK, 0, 0, 999, s.Get()); callback->OnFlush(DWORD(MF_SOURCE_READER_ALL_STREAMS)); }); late.join();
        expect(box->counts().late == 1 && box->counts().queued == 0, "cross-thread late read inert");
        expect(WaitForSingleObject(box->flushed(), 0) == WAIT_OBJECT_0, "flush acknowledgment");
        expect(WaitForSingleObject(box->retired(), 0) == WAIT_TIMEOUT, "flush is not callback release");
        callback.Reset(); expect(WaitForSingleObject(box->retired(), 0) == WAIT_OBJECT_0, "actual callback reference retired");
    }
    pass("production asynchronous callback, original QPC, late read and distinct retirement");
    {
        for (int restart = 0; restart < 3; ++restart) {
            CameraInput input;
            expect(input.status().phase == Phase::idle && !input.ready_event(), "constructor no device");
            expect(input.prepare(L"never-open-this-camera", 1, false) == E_ACCESSDENIED, "explicit denial before any activation");
            expect(input.prepare(L"", 1, true) == E_INVALIDARG, "empty source rejected before enumeration");
            HRESULT wrong = S_OK;
            std::thread other([&] { wrong = input.start(); }); other.join();
            expect(wrong == RPC_E_WRONG_THREAD, "owner thread enforcement");
            Frame f; expect(FAILED(input.read(f)) && f.bgr.empty(), "idle read no side effects");
            ok(input.stop(), "idle retirement"); expect(input.status().phase == Phase::stopped && !input.status().cleanup_uncertain, "actual empty retirement");
            expect(FAILED(input.start()), "stopped instance cannot restart");
        }
    }
    pass("three unopened production owners: denied/invalid selection and thread/stop boundaries");
}
}
int main() {
    const auto com = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(com)) return 2;
    const auto mf = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    if (FAILED(mf)) { CoUninitialize(); return 2; }
    int result = 0;
    try { run(); std::cout << "PASS " << groups << " groups, " << assertions << " assertions; memory samples only, no camera activation\n"; }
    catch (const std::exception& e) { std::cerr << "FAIL " << e.what() << '\n'; result = 1; }
    MFShutdown(); CoUninitialize(); return result;
}
