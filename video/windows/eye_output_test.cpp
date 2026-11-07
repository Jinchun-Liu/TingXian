#include "eye_output.h"
#include <array>
#include <iostream>
#include <stdexcept>

using namespace tingxian::video;
namespace {
int groups = 0;
void expect(bool ok, const char* message) { if (!ok) throw std::runtime_error(message); }
void check(HRESULT hr, const char* message) { expect(SUCCEEDED(hr), message); }
void pass(const char* message) { ++groups; std::cout << "PASS " << message << '\n'; }
template<class F> void wait(F predicate, const char* message) {
    const auto until = GetTickCount64() + 2500;
    while (GetTickCount64() < until) { if (predicate()) return; Sleep(1); }
    throw std::runtime_error(message);
}
tingxian::eye::Frame color(UINT64 generation, UINT64 id, BYTE b, BYTE g, BYTE r, UINT64 at = GetTickCount64()) {
    tingxian::eye::Frame f{generation, id, at, std::vector<BYTE>(size_t(Width) * Height * 3)};
    for (size_t i = 0; i < f.bgr.size(); i += 3) { f.bgr[i] = b; f.bgr[i+1] = g; f.bgr[i+2] = r; }
    return f;
}
void pixels(const std::vector<BYTE>& nv12, BYTE y, BYTE u, BYTE v) {
    expect(nv12.size() == FrameBytes, "NV12 length differs");
    for (size_t i = 0; i < nv12.size(); ++i)
        expect(nv12[i] == (i < size_t(Width) * Height ? y : (i % 2 ? v : u)), "NV12 pixel differs");
}
struct Handle {
    HANDLE value = INVALID_HANDLE_VALUE;
    ~Handle() { close(); }
    void close() { if (value != INVALID_HANDLE_VALUE) { CloseHandle(value); value = INVALID_HANDLE_VALUE; } }
};
// Independent wire observation: bounded reads, no MF/device/SDK. The slow-reader
// case intentionally consumes the frame header but not its payload.
struct Wire { UINT32 magic = 0x31565854; UINT16 version = 1, kind = 0; UINT32 bytes = 0, reserved = 0; UINT64 epoch = 0, id = 0, captured = 0; };
static_assert(sizeof(Wire) == 40);
struct Peer {
    Handle pipe;
    explicit Peer(const VideoPipeEndpoint& endpoint) {
        pipe.value = CreateFileW(endpoint.name, GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, nullptr);
        expect(pipe.value != INVALID_HANDLE_VALUE, "open independent peer");
        Wire hello; hello.kind = 1; hello.bytes = 32;
        DWORD done = 0;
        expect(WriteFile(pipe.value, &hello, sizeof(hello), &done, nullptr) && done == sizeof(hello), "hello");
        expect(WriteFile(pipe.value, endpoint.capability, 32, &done, nullptr) && done == 32, "capability");
        auto ack = header(); expect(ack.kind == 2 && !ack.bytes, "ack");
        auto begin = header(); expect(begin.kind == 3 && !begin.bytes, "begin");
    }
    void read(void* target, DWORD bytes) {
        DWORD total = 0;
        const auto until = GetTickCount64() + 2500;
        while (total < bytes) {
            expect(GetTickCount64() < until, "wire read timeout");
            DWORD available = 0, done = 0;
            expect(PeekNamedPipe(pipe.value, nullptr, 0, nullptr, &available, nullptr), "peer disconnected");
            if (!available) { Sleep(1); continue; }
            const DWORD chunk = (std::min)(available, bytes - total);
            expect(ReadFile(pipe.value, static_cast<BYTE*>(target) + total, chunk, &done, nullptr) && done, "wire read");
            total += done;
        }
    }
    Wire header() {
        Wire value; read(&value, sizeof(value));
        expect(value.magic == 0x31565854 && value.version == 1 && !value.reserved, "wire header differs");
        return value;
    }
};
void run() {
    {
        struct Case { BYTE b,g,r,y,u,v; };
        const Case cases[] = {{0,0,0,16,128,128},{255,255,255,235,128,128},
            {0,0,255,63,102,240},{0,255,0,173,42,26},{255,0,0,32,240,118},{128,128,128,126,128,128}};
        for (const auto& c : cases) {
            auto frame = color(1,1,c.b,c.g,c.r);
            const auto original = frame.bgr;
            std::vector<BYTE> out(FrameBytes);
            expect(corrected_bgr_to_nv12(frame.bgr.data(), frame.bgr.size(), out.data(), out.size()), "color conversion");
            pixels(out,c.y,c.u,c.v); expect(frame.bgr == original, "conversion changed input");
        }
        auto mixed = color(1,1,0,0,0);
        for (UINT32 y=0;y<Height;y+=2) for (UINT32 x=0;x<Width;x+=2) {
            const size_t p = (size_t(y)*Width+x)*3;
            mixed.bgr[p+2]=255; mixed.bgr[p+4]=255;
            mixed.bgr[p+Width*3]=255;
            std::fill_n(mixed.bgr.data()+p+Width*3+3,3,BYTE{255});
        }
        std::vector<BYTE> out(FrameBytes,201);
        expect(!corrected_bgr_to_nv12(mixed.bgr.data(),mixed.bgr.size()-1,out.data(),out.size()), "short input accepted");
        expect(out == std::vector<BYTE>(FrameBytes,201), "invalid conversion touched destination");
        expect(corrected_bgr_to_nv12(mixed.bgr.data(),mixed.bgr.size(),out.data(),out.size()), "mixed block conversion");
        for (size_t i=size_t(Width)*Height;i<out.size();++i) expect(out[i]==128,"chroma not averaged before quantization");
        pass("BT709 color bars, centered mixed-block chroma, immutable input and exact lengths");
    }
    {
        EyePipeOutput sink; VideoPipeEndpoint endpoint{};
        expect(!sink.corrected(color(1,1,0,0,255)),"output enabled by construction");
        const auto cancelled = sink.operation_ticket(); sink.request_stop();
        expect(FAILED(sink.open_if_current(1,cancelled,&endpoint)),"late start crossed stop");
        check(sink.open_if_current(1,sink.operation_ticket(),&endpoint),"explicit open");
        for (UINT64 i=1;i<=10;++i) expect(sink.corrected(color(1,i,0,0,255)),"bounded pre-connect handoff");
        auto s=sink.status(); expect(s.queued==1 && s.accepted==10 && s.dropped==9,"queue was not latest-only");
        expect(!sink.corrected(color(1,10,0,0,255)),"duplicate id accepted");
        expect(!sink.corrected(color(2,11,0,0,255)),"wrong generation accepted");
        expect(!sink.corrected(color(1,11,0,0,255,GetTickCount64()+10000)),"future timestamp accepted");
        expect(!sink.corrected(color(1,11,0,0,255,GetTickCount64()-1000)),"stale timestamp accepted");
        const auto start=GetTickCount64(); sink.neutral(2); sink.close();
        expect(GetTickCount64()-start<1000 && sink.status().queued==0,"pending accept did not stop");
        expect(FAILED(sink.open_if_current(1,sink.operation_ticket(),&endpoint)),"old generation restarted");
        pass("disabled by default, pre-dispatch cancellation, bounded queue and stale admission rejection");
    }
    {
        const UINT64 generation=1;
        EyePipeOutput sink; VideoPipeEndpoint endpoint{};
        check(sink.open_if_current(generation,sink.operation_ticket(),&endpoint),"wire open");
        Peer peer(endpoint);
        auto frame=color(generation,73,0,0,255,GetTickCount64()-60);
        expect(sink.corrected(frame),"wire handoff");
        auto h=peer.header();
        expect(h.kind==4 && h.bytes==FrameBytes && h.epoch==generation && h.id==73
            && h.captured==frame.captured_ms,"sender rewrote frame identity or capture time");
        std::vector<BYTE> out(FrameBytes); peer.read(out.data(),FrameBytes); pixels(out,63,102,240);
        wait([&]{return sink.status().submitted==1;},"handoff not accounted");
        sink.close(); expect(!sink.corrected(frame),"closed sink accepted frame");
    }
    pass("actual authenticated pipe carries corrected pixels and original frame identity/time");
    {
        EyePipeOutput sink; VideoPipeEndpoint endpoint{};
        check(sink.open_if_current(1,sink.operation_ticket(),&endpoint),"delayed peer open");
        expect(sink.corrected(color(1,1,0,0,255)),"delayed peer pending frame");
        Sleep(DWORD(MaxFrameAgeMs+20)); // the receiver has not connected yet
        Peer peer(endpoint);
        wait([&]{return sink.status().dropped==1 && !sink.status().busy;},"queued stale frame was not dropped");
        DWORD available=0;
        expect(PeekNamedPipe(peer.pipe.value,nullptr,0,nullptr,&available,nullptr) && available==0,
            "expired pending frame reached the wire");
        expect(sink.corrected(color(1,2,255,255,255)),"fresh frame after pending expiry");
        auto h=peer.header(); expect(h.kind==4 && h.id==2,"expired frame was relabeled as fresh");
        std::vector<BYTE> out(FrameBytes);peer.read(out.data(),FrameBytes);pixels(out,235,128,128);
        sink.close();
        pass("delayed connection drops expired pending pixels without renewing their capture time");
    }
    {
        EyePipeOutput sink; VideoPipeEndpoint endpoint{};
        for (UINT64 gen=1;gen<=3;++gen) {
            check(sink.open_if_current(gen,sink.operation_ticket(),&endpoint),"restart open");
            Peer peer(endpoint);
            if (gen>1) sink.neutral(gen-1);
            auto frame=color(gen,1,255,255,255);
            expect(sink.corrected(frame),"explicit restart handoff");
            auto h=peer.header(); expect(h.kind==4 && h.epoch==gen,"old generation after restart");
            std::vector<BYTE> out(FrameBytes); peer.read(out.data(),FrameBytes); pixels(out,235,128,128);
            sink.neutral(gen); sink.close();
        }
        pass("three explicit restarts isolate old neutral events and old frames");
    }
    {
        EyePipeOutput sink; VideoPipeEndpoint endpoint{};
        check(sink.open_if_current(1,sink.operation_ticket(),&endpoint),"slow peer open");
        Peer peer(endpoint);
        expect(sink.corrected(color(1,1,0,0,255)),"slow first frame");
        expect(peer.header().kind==4,"missing slow frame header");
        // Deliberately leave >1 MiB of payload unread in the 128 KiB pipe.
        for (UINT64 i=2;i<=8;++i) expect(sink.corrected(color(1,i,0,0,255)),"slow queued handoff");
        auto s=sink.status(); expect(s.queued<=1 && s.busy && s.dropped>=5,"slow peer accumulated frames");
        const auto start=GetTickCount64(); sink.neutral(1);
        expect(GetTickCount64()-start<500,"sink callback waited for pipe I/O");
        sink.close(); expect(GetTickCount64()-start<1000,"slow writer did not retire");
        expect(!sink.status().queued && !sink.status().busy,"stop retained frames");
        pass("slow receiver has one pending frame; stop interrupts in-flight I/O without callback blocking");
    }
    {
        EyePipeOutput sink; VideoPipeEndpoint endpoint{};
        check(sink.open_if_current(1,sink.operation_ticket(),&endpoint),"fault peer open");
        Peer peer(endpoint); peer.pipe.close();
        expect(sink.corrected(color(1,1,0,0,255)),"fault discovery handoff");
        wait([&]{return sink.status().state==EyeOutputState::fault;},"broken peer not surfaced");
        expect(!sink.corrected(color(1,2,0,0,255)),"fault auto resumed");
        expect(FAILED(sink.open_if_current(2,sink.operation_ticket(),&endpoint)),"fault skipped cleanup");
        sink.close();
        pass("transport failure remains visible and cannot resume without explicit cleanup/start");
    }
}
}
int main() {
    try { run(); std::cout << "{\"checks\":" << groups << ",\"synthetic_only\":true,\"device_registered\":false}\n"; return 0; }
    catch(const std::exception& e) { std::cerr<<"FAIL "<<e.what()<<'\n'; return 1; }
}
