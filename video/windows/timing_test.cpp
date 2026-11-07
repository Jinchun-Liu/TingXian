// Deterministic scheduling only: no media device, MF registration or playback.
#include "frame_mailbox.h"
#include <iostream>
#include <stdexcept>
#include <thread>
#include <atomic>
using namespace tingxian::video;
namespace {
int groups = 0;
void expect(bool value, const char* reason) { if (!value) throw std::runtime_error(reason); }
void check(HRESULT hr, const char* reason) { expect(SUCCEEDED(hr), reason); }
void pass(const char* name) { ++groups; std::cout << "PASS " << name << '\n'; }
constexpr UINT64 Now = 100000000;
struct Fixture {
    FrameMailbox box;
    UINT64 generation = 0;
    std::vector<BYTE> pixels = std::vector<BYTE>(FrameBytes, 96), out = std::vector<BYTE>(FrameBytes);
    TimedSample stamp;
    LONGLONG pts = 0;
    bool timed = false;
    Fixture() { std::fill_n(pixels.begin(), Width * Height, BYTE{160}); check(box.begin_timed(1, &generation), "begin"); }
    HRESULT send(UINT64 seq, TimedFrame t, UINT64 now = Now, UINT64 gen = 0) {
        const bool neutral = t.flags == TimedNeutral;
        return box.submit_timed(gen ? gen : generation, seq, &t, neutral ? nullptr : pixels.data(), neutral ? 0 : FrameBytes, now);
    }
    void sample(UINT64 now, bool fail = false) {
        auto hr = box.write_sample(out.data(), Width, out.data(), FrameBytes, now, pts,
            [&](const TimedSample* value, LONGLONG time) {
                timed = value != nullptr;
                if (fail) return E_FAIL;
                if (value) stamp = *value;
                pts = time; return S_OK;
            });
        expect(fail ? FAILED(hr) : SUCCEEDED(hr), "handoff result");
    }
    void color(BYTE y, BYTE uv) {
        expect(std::all_of(out.begin(), out.begin() + Width * Height, [=](BYTE x){return x == y;}) &&
            std::all_of(out.begin() + Width * Height, out.end(), [=](BYTE x){return x == uv;}), "pixel mismatch");
    }
};
void run() {
    {
        Fixture f; TimedFrame t{7, Now - 400000, Now + 200000};
        check(f.send(1, t), "future submit"); f.sample(Now); f.color(32,128);
        expect(f.timed && f.stamp.sequence == 0 && f.stamp.capture_qpc == 0, "future leaked into sample");
        f.sample(t.presentation_qpc + 10); f.color(160,96);
        expect(f.pts == LONGLONG(t.presentation_qpc) && f.stamp.capture_qpc == t.capture_qpc &&
            f.stamp.frame_id == 7 && f.stamp.sequence == 1 && f.stamp.flags == 0, "planned identity lost");
        f.sample(t.presentation_qpc + 20); f.color(160,96);
        expect(f.stamp.flags == TimedRepeated && f.stamp.capture_qpc == t.capture_qpc &&
            f.stamp.presentation_qpc == t.presentation_qpc, "repeat refreshed capture or plan");
        f.sample(t.presentation_qpc + FrameDuration); f.color(32,128);
        auto m=f.box.timed_metrics(); expect(m.unique==1 && m.repeated==1 && m.neutral==2, "sample counts");
        pass("future gating, exact planned PTS, original capture, repeat and expiry");
    }
    {
        Fixture f;
        for(UINT64 i=1;i<=4;++i) check(f.send(i,{i,Now,Now+i*100000}),"queue fill");
        expect(f.send(5,{5,Now,Now+500000})==HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW),"queue unbounded");
        expect(f.box.timed_metrics().queued==4,"queue capacity");
        f.sample(Now+400000); f.color(160,96);
        auto m=f.box.timed_metrics(); expect(m.queued==0 && m.superseded==3 && m.unique==1 && m.rejected==1,"latest due accounting");
        pass("four-slot capacity and obsolete due frames counted");
    }
    {
        Fixture f;
        TimedFrame t{1,Now,Now+100000}; check(f.send(1,t),"first");
        expect(FAILED(f.send(2,{1,Now+1,Now+200000},Now+1)),"same frame capture refreshed");
        expect(FAILED(f.send(2,{2,Now-1,Now+200000})),"capture regressed");
        expect(FAILED(f.send(1,{2,Now,Now+200000})),"sequence reused");
        expect(FAILED(f.send(2,{2,Now,Now+100000})),"plan reused");
        expect(FAILED(f.send(2,{2,Now,Now+TimedHorizon+1})),"unbounded lead");
        expect(FAILED(f.send(2,{2,Now+1,Now+200000})),"future capture");
        expect(FAILED(f.send(2,{2,Now-TimedMaxAge-1,Now+200000})),"old capture");
        expect(FAILED(f.send(2,{2,Now,UINT64(LLONG_MAX)})),"timestamp overflow");
        expect(FAILED(f.send(2,{0,0,Now+200000,TimedNeutral,1})),"reserved bits");
        check(f.send(2,{1,Now,Now+200000}),"same original frame allowed at new plan");
        pass("timestamp identity, replay, age, lead, overflow and reserved-field checks");
    }
    {
        Fixture f; f.sample(Now+100000);
        check(f.send(1,{1,Now,Now+50000},Now+100000),"late relative to sample");
        f.sample(Now+150000); f.color(32,128);
        expect(f.box.timed_metrics().late==1,"missed presentation relabelled");
        check(f.send(2,{2,Now,Now+200000},Now+150000),"next plan");
        f.sample(Now+200000,true);
        expect(f.box.timed_metrics().unique==0,"failed callback counted delivered");
        f.sample(Now+200001); f.color(160,96);
        expect(f.pts==LONGLONG(Now+200000) && f.box.timed_metrics().unique==1,"handoff retry changed plan");
        pass("late samples drop without retiming; failed handoff remains uncommitted");
    }
    {
        Fixture f; check(f.send(1,{1,Now,Now}),"media"); f.sample(Now); f.color(160,96);
        check(f.send(2,{0,0,Now+100000,TimedNeutral}),"scheduled neutral");
        f.sample(Now+100000); f.color(32,128);
        expect(f.stamp.flags==TimedNeutral && f.stamp.sequence==2 && !f.stamp.capture_qpc &&
            f.pts==LONGLONG(Now+100000),"neutral plan fabricated capture");
        const auto old=f.generation; f.box.stop(); f.sample(Now+100001); f.color(32,128);
        expect(FAILED(f.send(3,{3,Now,Now+200000},Now+100001,old)),"stopped generation accepted");
        expect(FAILED(f.box.begin_timed(1,&f.generation)),"epoch reused");
        for(UINT64 epoch=2;epoch<=4;++epoch){
            check(f.box.begin_timed(epoch,&f.generation),"restart");
            expect(f.generation>old,"generation reused");
            check(f.send(1,{1,Now,Now+200000},Now+100001),"restart frame"); f.box.stop();
        }
        expect(f.box.timed_metrics().queued==0 && f.box.timed_metrics().cleared==3,"stop retained queues");
        pass("scheduled neutral, three restarts and stale generation isolation");
    }
    {
        Fixture f; check(f.send(1,{1,Now,Now}),"first plan"); f.sample(Now);
        check(f.send(2,{1,Now,Now+100000}),"same frame next plan"); f.sample(Now+100000);
        expect(f.stamp.flags==TimedRepeated && f.pts==LONGLONG(Now+100000) &&
            f.stamp.capture_qpc==Now,"new plan changed capture or repeated identity");
        const auto m=f.box.timed_metrics(); expect(m.unique==1 && m.repeated==1,"same pixels inflated unique FPS");
        pass("new scheduled slot preserves unique versus repeated frame accounting");
    }
    {
        Fixture f; UINT64 legacy=0;
        expect(FAILED(f.box.begin(&legacy)),"v1 took active v2");
        expect(FAILED(f.box.submit(f.generation,1,GetTickCount64(),f.pixels.data(),FrameBytes)),"v1 submitted into v2");
        f.box.stop(); check(f.box.begin(&legacy),"explicit legacy restart");
        expect(!f.box.timed_metrics().enabled && f.box.metrics().enabled,"legacy activity labelled timed-ready");
        expect(FAILED(f.box.begin_timed(2,&f.generation)),"v2 took active v1");
        check(f.box.submit(legacy,1,GetTickCount64(),f.pixels.data(),FrameBytes),"legacy submit");
        f.sample(Now); f.color(160,96); expect(!f.timed,"legacy sample retained v2 metadata");
        pass("mode switch requires stop and legacy path retains behavior");
    }
    {
        Fixture f; check(f.send(1,{1,Now,Now+100000}),"expiry queued");
        f.sample(Now+500000); f.color(32,128);
        expect(f.box.timed_metrics().late==1 && !f.box.timed_metrics().queued,"expired queue retained");
        TimedFrame aged{2,Now-TimedMaxAge,Now+1};
        expect(FAILED(f.send(2,aged)),"plan extended capture lease");
        pass("unrequested plans expire and cannot extend capture lease");
    }
    {
        Fixture f; check(f.send(1,{1,Now,Now}),"concurrent frame");
        std::atomic<bool> inside{false}, release{false}, stopped{false}, attempting{false};
        HRESULT delivered=E_FAIL;
        std::thread writer([&]{delivered=f.box.write_sample(f.out.data(),Width,f.out.data(),FrameBytes,Now,0,
            [&](const TimedSample*,LONGLONG){inside=true;while(!release.load())Sleep(1);return S_OK;});});
        const auto deadline=GetTickCount64()+2000;
        while(!inside.load()&&GetTickCount64()<deadline)Sleep(1);
        std::thread stopper([&]{attempting=true;f.box.stop();stopped=true;});
        while(!attempting.load()&&GetTickCount64()<deadline)Sleep(1);
        const bool ordered=inside.load() && !stopped.load(); release=true; writer.join(); stopper.join();
        expect(ordered&&SUCCEEDED(delivered)&&stopped.load(),"stop and handoff race");
        f.sample(Now+1); f.color(32,128); pass("stop serializes with sample handoff and clears afterward");
    }
}
}
int main(){try{run();std::cout<<"{\"checks\":"<<groups<<",\"result\":\"pass\",\"synthetic_only\":true}\n";return 0;}
catch(const std::exception& e){std::cerr<<"FAIL "<<e.what()<<'\n';return 1;}}
