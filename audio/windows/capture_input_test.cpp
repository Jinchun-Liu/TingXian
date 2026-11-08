// Synthetic packet/clock backend. Never constructs WindowsInput or opens audio.
#include "capture_input.h"
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <algorithm>
#include <cmath>
#include <iostream>
#include <stdexcept>
#include <thread>
using namespace tingxian::capture;
using tingxian::audio::Endpoint;
using tingxian::audio::Flow;
namespace {
unsigned checks = 0;
void require(bool value, const char* label) {
    ++checks; if (!value) throw std::runtime_error(label);
}
std::vector<Endpoint> inventory() {
    Endpoint mic; mic.id = L"selected-mic"; mic.flow = Flow::capture; mic.active = true; mic.form_factor = Microphone;
    return {mic};
}
const Selection selected{L"selected-mic", L"output-return"};
struct Memory final : Backend {
    std::vector<Endpoint> endpoints = inventory();
    Format format{rate, 1, max_frames};
    std::array<float, max_frames> samples{};
    RawPacket packet{samples.data(), 480, 0, 12345, 100000000};
    uint64_t clock = 101000000;
    uint32_t inspected = 0, prepared = 0, started = 0, acquired = 0, released = 0, released_frames = 0, stopped = 0;
    HRESULT prepare_error = S_OK, start_error = S_OK, acquire_error = S_OK, release_error = S_OK, clock_error = S_OK;
    Cleanup cleanup{S_OK, S_OK, true};
    bool held = false;
    Memory() { for (size_t i = 0; i < samples.size(); ++i) samples[i] = (i % 3 == 0 ? -0.25f : 0.5f); }
    HRESULT inspect(const Selection& s) noexcept override { ++inspected; return validate_selection(endpoints, s); }
    HRESULT prepare(const Selection& s, Format& out) noexcept override {
        ++prepared; const auto hr = validate_selection(endpoints, s);
        if (hr != S_OK) return hr;
        out = format; return prepare_error;
    }
    HRESULT start() noexcept override { ++started; return start_error; }
    HRESULT acquire(RawPacket& out) noexcept override {
        ++acquired;
        if (held) return AUDCLNT_E_OUT_OF_ORDER;
        if (acquire_error != S_OK) return acquire_error;
        out = packet; held = packet.frames != 0; return S_OK;
    }
    HRESULT release(uint32_t frames) noexcept override {
        ++released; released_frames = frames;
        if (!held || frames != packet.frames) return E_UNEXPECTED;
        held = false; return release_error;
    }
    HRESULT now(uint64_t& out) noexcept override { out = clock; return clock_error; }
    Cleanup stop_reset() noexcept override { ++stopped; held = false; return cleanup; }
};
void begin(Session& s) {
    require(s.prepare(selected, 7, true) == S_OK, "prepare");
    require(s.start(7) == S_OK, "start");
}
bool cleared(const Packet& p) { return !p.frames && !p.generation && std::all_of(p.samples.begin(), p.samples.end(), [](float s){return s==0;}); }
void selection_and_authority() {
    for (unsigned mode = 0; mode < 8; ++mode) {
        Memory m; Session s(m);
        Selection choice = selected;
        if (mode == 1) choice.microphone_id = choice.output_return_id;
        if (mode == 2) m.endpoints[0].flow = Flow::render;
        if (mode == 3) m.endpoints[0].active = false;
        if (mode == 4) m.endpoints[0].form_factor = UnknownFormFactor;
        if (mode == 5) m.endpoints.push_back(m.endpoints[0]);
        if (mode == 6) { m.endpoints[0].adapter_id=L"cable";m.endpoints[0].hardware_id=L"VBAudioVACWDM";m.endpoints[0].service=L"VBAudioVACMME"; }
        if (mode == 7) choice.microphone_id = L"missing";
        require(FAILED(s.prepare(choice, 7, mode != 0)), "source/permission must reject");
        require(!m.prepared && !m.started, "denied capture initialized backend");
        if (!mode) require(!m.inspected, "permission denial enumerated");
    }
    Memory m; Session s(m); begin(s);
    Packet p;
    require(FAILED(s.read(8,p)) && !m.acquired, "stale generation read OS buffer");
    HRESULT foreign = S_OK;
    std::thread other([&]{Packet local; foreign=s.read(7,local);});other.join();
    require(foreign==RPC_E_WRONG_THREAD && !m.acquired, "wrong thread read");
    require(s.stop()==S_OK, "stop");
    require(FAILED(s.start(7)) && FAILED(s.prepare(selected,8,true)), "stopped session restarted");
}
void exact_pcm_and_flags() {
    Memory m; Session s(m); begin(s); Packet p;
    m.packet.frames = max_frames; m.packet.flags = AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY;
    require(s.read(7,p)==S_OK, "first packet boundary");
    require(p.frames==max_frames && p.first_frame==12345 && p.qpc_100ns==100000000 && p.generation==7, "original metadata changed");
    require(p.flags==AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY && s.status().startup_discontinuities==1, "initial flag lost");
    require(p.samples==m.samples && m.released==1 && m.released_frames==max_frames && !m.held, "PCM/release changed");
    m.packet.first_frame+=max_frames;m.packet.qpc_100ns+=1000000;m.clock+=1000000;
    m.packet.frames=480;m.packet.flags=AUDCLNT_BUFFERFLAGS_SILENT;m.packet.samples=nullptr;
    require(s.read(7,p)==S_OK && p.flags==AUDCLNT_BUFFERFLAGS_SILENT, "silent/null packet");
    require(std::all_of(p.samples.begin(),p.samples.end(),[](float f){return f==0;}), "silence was not zero");
    require(m.released==2 && s.status().delivered_frames==max_frames+480, "frame accounting");
    m.acquire_error=S_FALSE;
    require(s.read(7,p)==S_FALSE && cleared(p) && m.released==2, "empty packet consumed previous data");
    require(s.stop()==S_OK && s.status().cleanup.reset && m.stopped==1, "cleanup");
}
void rejected_packets_release_before_fault() {
    for (unsigned mode = 0; mode < 13; ++mode) {
        Memory m; Session s(m); begin(s); Packet p;
        if (mode==0) m.packet.frames=max_frames+1;
        if (mode==1) m.packet.flags=AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR;
        if (mode==2) m.packet.flags=0x8000;
        if (mode==3) m.packet.qpc_100ns=m.clock+1;
        if (mode==4) m.packet.qpc_100ns=m.clock-max_age-1;
        if (mode==5) m.packet.qpc_100ns=m.clock; // Last sample is in the future.
        if (mode==6) m.packet.first_frame=UINT64_MAX-2;
        if (mode==7) m.samples[17]=(std::numeric_limits<float>::quiet_NaN)();
        if (mode==8) m.packet.samples=nullptr;
        if (mode==9) m.clock_error=E_FAIL;
        if (mode==10) m.release_error=E_ACCESSDENIED;
        if (mode==11) m.packet.qpc_100ns=0;
        if (mode==12) m.packet.frames=0;
        require(FAILED(s.read(7,p)), "bad packet accepted");
        require(cleared(p) && !s.status().delivered_frames, "bad PCM exposed");
        require(m.released==(mode==12?0u:1u) && m.stopped==1, "missing/duplicate release");
        require(s.status().phase==Phase::fault, "failure did not fault");
        if (mode==10) require(s.status().release_error==E_ACCESSDENIED, "release error lost");
        require(FAILED(s.read(7,p)) && m.acquired==1, "fault continued capture");
    }
}
void continuity_and_cleanup() {
    for (unsigned mode=0;mode<5;++mode) {
        Memory m;Session s(m);begin(s);Packet p;require(s.read(7,p)==S_OK,"initial packet");
        m.packet.first_frame+=480;m.packet.qpc_100ns+=100000;m.clock+=100000;
        if(mode==0)m.packet.first_frame++;
        if(mode==1)m.packet.first_frame-=480;
        if(mode==2)m.packet.qpc_100ns-=100000;
        if(mode==3)m.packet.qpc_100ns+=clock_residual+1;
        if(mode==4)m.packet.flags=AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY;
        require(FAILED(s.read(7,p)) && cleared(p),"continuity accepted");
        require(m.released==2 && s.status().delivered_frames==480 && s.status().discarded_frames==480,"gap accounting");
    }
    for (unsigned mode=0;mode<3;++mode) {
        Memory m;Session s(m);begin(s);
        m.cleanup.reset=false;
        if(mode==0)m.cleanup.stop_error=E_ACCESSDENIED;
        if(mode==1)m.cleanup.reset_error=E_FAIL;
        require(FAILED(s.stop()) && s.status().phase==Phase::fault,"unconfirmed cleanup accepted");
        require(FAILED(s.stop()) && m.stopped==1,"cleanup failure forgotten");
    }
    for(unsigned mode=0;mode<4;++mode) {
        Memory m;Session s(m);begin(s);
        m.cleanup={mode==0?S_OK:S_FALSE, mode<2?S_OK:S_FALSE, true};
        if(mode==3)m.cleanup.stop_error=7;
        require((mode==3?FAILED(s.stop()):s.stop()==S_OK),"documented cleanup return semantics");
        require(s.status().cleanup.stop_error==m.cleanup.stop_error && s.status().cleanup.reset_error==m.cleanup.reset_error,"raw cleanup results lost");
    }
    for(unsigned mode=0;mode<4;++mode) {
        Memory m;Session s(m);
        if(mode==0)m.prepare_error=E_FAIL;
        if(mode==1)m.format.rate=44100;
        if(mode==2)m.format.channels=2;
        if(mode==3)m.format.buffer_frames=max_frames+1;
        require(FAILED(s.prepare(selected,7,true)) && m.stopped==1 && !m.started,"failed prepare leaked");
    }
    for(unsigned i=0;i<3;++i){Memory m;Session s(m);begin(s);Packet p;require(s.read(7,p)==S_OK && p.first_frame==12345,"new lifetime retained old origin");require(s.stop()==S_OK,"new lifetime stop");}
}
void runtime_failures() {
    for(unsigned mode=0;mode<4;++mode) {
        Memory m;Session s(m);Packet p;
        require(s.prepare(selected,7,true)==S_OK,"runtime prepare");
        if(mode==0)m.start_error=E_ACCESSDENIED;
        if(mode==0) require(s.start(7)==E_ACCESSDENIED && !m.acquired,"start permission failure ignored");
        else {
            require(s.start(7)==S_OK,"runtime start");
            if(mode==1)m.acquire_error=AUDCLNT_E_DEVICE_INVALIDATED;
            if(mode==2)m.acquire_error=7;
            if(mode==3)m.clock_error=S_FALSE;
            require(FAILED(s.read(7,p)) && cleared(p),"runtime failure accepted");
        }
        require(s.status().phase==Phase::fault && m.stopped==1,"runtime failure did not clean up");
        require(m.released==(mode==3?1u:0u),"failure released unacquired packet");
    }
}
}
int main() {
    try { selection_and_authority();exact_pcm_and_flags();rejected_packets_release_before_fault();continuity_and_cleanup();runtime_failures();
        std::cout<<"PASS capture memory: "<<checks<<" assertions; original PCM/QPC, source, bounded packets, release, generation and cleanup; no device opened\n";
        return 0;
    } catch(const std::exception& e){std::cerr<<"FAIL capture memory: "<<e.what()<<'\n';return 1;}
}
