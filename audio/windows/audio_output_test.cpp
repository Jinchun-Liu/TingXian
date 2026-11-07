#include "audio_output.h"
#include <algorithm>
#include <iostream>
#include <limits>
#include <stdexcept>
#include <thread>
#include <array>

using namespace tingxian::audio;
namespace {
void check(bool value, const char* message) { if (!value) throw std::runtime_error(message); }
std::vector<Endpoint> inventory() {
    return {{L"render-1", L"Arbitrary name", L"ROOT\\CABLE\\0001", L"VBAudioVACWDM", L"VBAudioVACMME", Flow::render, true},
            {L"capture-1", L"Unrelated label", L"ROOT\\CABLE\\0001", L"VBAudioVACWDM", L"VBAudioVACMME", Flow::capture, true}};
}
const Pair pair{L"render-1", L"capture-1"};
struct Fake final : Backend {
    Format negotiated{48000, 2, 1920, 192000}; // Four clock units per sample frame.
    Snapshot now{0, 0, 10000000};
    HRESULT prepare_hr = S_OK, start_hr = S_OK, snapshot_hr = S_OK, write_hr = S_OK;
    Cleanup cleanup{S_OK, S_OK, true, true}; // Only this in-memory backend verifies its own drain.
    int prepares = 0, starts = 0, writes = 0, stops = 0;
    bool primed = false;
    std::vector<float> pcm;
    HRESULT prepare(const Pair&, uint32_t, Format& out) noexcept override {
        ++prepares; out = negotiated; now = {0, 0, 10000000}; return prepare_hr;
    }
    HRESULT start() noexcept override { ++starts; return primed ? start_hr : E_UNEXPECTED; }
    HRESULT snapshot(Snapshot& out) noexcept override { out = now; return snapshot_hr; }
    HRESULT write_mono(const float* input, uint32_t frames, bool silence) noexcept override {
        ++writes;
        if (FAILED(write_hr)) return write_hr;
        if (frames > negotiated.buffer_frames - now.padding) return E_FAIL;
        const auto old = pcm.size();
        pcm.resize(old + frames * negotiated.channels, 0);
        if (!silence) {
            const auto hr = copy_mono_interleaved(input, frames, negotiated.channels, pcm.data() + old,
                                                frames * negotiated.channels);
            if (FAILED(hr)) return hr;
        }
        primed = primed || silence;
        now.padding += frames;
        return S_OK;
    }
    Cleanup stop_reset() noexcept override {
        ++stops;
        if (cleanup.client_reset) { now = {}; pcm.clear(); primed = false; }
        return cleanup;
    }
    void consume(uint32_t frames) {
        now.position += static_cast<uint64_t>(frames) * 4;
        now.qpc_100ns += static_cast<uint64_t>(frames) * 10000000 / sample_rate;
        now.padding -= std::min(now.padding, frames);
    }
};
struct Lease final : OutputLease {
    int revoked = 0;
    void revoke() noexcept override { ++revoked; }
};
struct Fixture {
    Fake backend;
    Lease lease;
    Session session{backend, lease};
    std::array<float, 480> voice{};
    Fixture() { for (size_t i = 0; i < voice.size(); ++i) voice[i] = static_cast<float>(static_cast<int>(i % 13) - 6) / 8; }
    void run(uint64_t generation = 1) {
        check(session.prepare(inventory(), pair, 2, generation) == S_OK, "prepare");
        check(session.start(generation, true) == S_OK, "start");
    }
    Ticket reserve(uint64_t generation = 1) {
        Ticket t;
        check(session.reserve(generation, 480, t) == S_OK, "reserve");
        return t;
    }
    void faulted() {
        check(session.status().phase == Phase::fault && lease.revoked && backend.stops, "fail-closed cleanup");
    }
};
void identity() {
    auto endpoints = inventory();
    check(validate_pair(endpoints, pair) == S_OK, "pair ignores friendly names");
    for (int variant = 0; variant < 7; ++variant) {
        endpoints = inventory();
        switch (variant) {
        case 0: endpoints[0].hardware_id = L"PhysicalSpeaker"; break;
        case 1: endpoints[1].adapter_id = L"ROOT\\CABLE\\0002"; break;
        case 2: endpoints[1].service = L"UnknownService"; break;
        case 3: endpoints[1].flow = Flow::render; break;
        case 4: endpoints[1].active = false; break;
        case 5: endpoints.erase(endpoints.begin()); break;
        case 6: endpoints.push_back(endpoints[0]); break;
        }
        Fixture f;
        check(FAILED(f.session.prepare(endpoints, pair, 2, 1)) && f.backend.prepares == 0, "reject before activate");
    }
    check(FAILED(validate_pair(inventory(), {L"render-1", L"render-1"})), "distinct ids");
}
void default_off() {
    Fixture f;
    check(f.backend.prepares == 0 && f.backend.starts == 0, "constructor side effects");
    check(f.session.prepare(inventory(), pair, 2, 1) == S_OK, "prepare");
    check(FAILED(f.session.start(1, false)) && f.backend.starts == 0 && f.backend.writes == 0, "explicit intent");
    check(FAILED(f.session.start(2, true)), "wrong generation cannot start");
    check(f.session.start(1, true) == S_OK && f.backend.primed, "prime before start");
    check(std::all_of(f.backend.pcm.begin(), f.backend.pcm.end(), [](float s){ return s == 0; }), "prime silence");
}
void formats() {
    for (int variant = 0; variant < 6; ++variant) {
        Fixture f;
        switch (variant) {
        case 0: f.backend.negotiated.rate = 44100; break;
        case 1: f.backend.negotiated.channels = 6; break;
        case 2: f.backend.negotiated.buffer_frames = 4801; break;
        case 3: f.backend.negotiated.buffer_frames = 959; break;
        case 4: f.backend.negotiated.clock_frequency = 0; break;
        case 5: f.backend.negotiated.clock_frequency = 1000000001; break;
        }
        check(FAILED(f.session.prepare(inventory(), pair, 2, 1)), "exact format required");
        check(f.backend.starts == 0, "no fallback"); f.faulted();
    }
}
void pcm_and_clock() {
    Fixture f; f.run(); f.backend.consume(480);
    const auto t = f.reserve();
    check(t.first_frame == 960 && t.clock.position == 1920 && t.clock.padding == 480, "consumption != writes");
    check(f.session.commit(t, Source::self_microphone, f.voice.data(), 480) == S_OK, "voice write");
    check(f.session.status().submitted_frames == 1440, "submitted watermark");
    for (size_t i = 0; i < f.voice.size(); ++i)
        check(f.backend.pcm[1920 + 2*i] == f.voice[i] && f.backend.pcm[1921 + 2*i] == f.voice[i], "same real voice twice");
    check(FAILED(f.session.commit(t, Source::self_microphone, f.voice.data(), 480)), "ticket single-use");
}
void backpressure() {
    Fixture f; f.run();
    auto t = f.reserve();
    Ticket second;
    check(FAILED(f.session.reserve(1, 480, second)), "one reservation");
    check(f.session.commit(t, Source::self_microphone, f.voice.data(), 480) == S_OK, "write 1");
    t = f.reserve();
    check(f.session.commit(t, Source::self_microphone, f.voice.data(), 480) == S_OK, "write 2");
    check(f.session.reserve(1, 480, second) == S_FALSE && second.serial == 0 && f.backend.writes == 3,
          "full queue no write or ticket");
    check(FAILED(f.session.reserve(1, 961, second)), "bounded write");
    f.backend.consume(480);
    check(f.session.reserve(1, 480, second) == S_OK, "consumption frees capacity");
}
void wrong_source_or_pcm() {
    for (int variant = 0; variant < 6; ++variant) {
        Fixture f; f.run(); auto t = f.reserve();
        Source source = Source::self_microphone;
        if (variant < 3) source = static_cast<Source>(variant + 1);
        if (variant == 3) f.voice[4] = std::numeric_limits<float>::quiet_NaN();
        if (variant == 4) f.voice[4] = 1.01f;
        if (variant == 5) ++t.clock.qpc_100ns;
        check(FAILED(f.session.commit(t, source, f.voice.data(), 480)) && f.backend.writes == 1, "bad voice not handed off");
        f.faulted();
    }
}
void late_underrun_clock() {
    for (int variant = 0; variant < 5; ++variant) {
        Fixture f; f.run(); auto t = f.reserve();
        switch (variant) {
        case 0: f.backend.consume(900); break;
        case 1: f.backend.consume(961); break;
        case 2: f.backend.now.qpc_100ns += 200001; break;
        case 3: --f.backend.now.qpc_100ns; break;
        case 4: f.backend.snapshot_hr = S_FALSE; break;
        }
        check(FAILED(f.session.commit(t, Source::self_microphone, f.voice.data(), 480)) && f.backend.writes == 1,
              "late/invalid clock not written");
        f.faulted();
    }
}
void failures() {
    for (int variant = 0; variant < 4; ++variant) {
        Fixture f;
        if (variant == 0) {
            f.backend.prepare_hr = E_FAIL;
            check(FAILED(f.session.prepare(inventory(), pair, 2, 1)), "prepare failure");
        } else if (variant == 1) {
            check(f.session.prepare(inventory(), pair, 2, 1) == S_OK, "prepare");
            f.backend.start_hr = E_ACCESSDENIED;
            check(FAILED(f.session.start(1, true)), "start failure");
        } else {
            f.run(); auto t = f.reserve();
            if (variant == 2) f.backend.snapshot_hr = HRESULT_FROM_WIN32(ERROR_DEVICE_NOT_CONNECTED);
            if (variant == 3) f.backend.write_hr = E_FAIL;
            check(FAILED(f.session.commit(t, Source::self_microphone, f.voice.data(), 480)), "write/device failure");
        }
        f.faulted(); check(f.backend.pcm.empty(), "fake queue drained");
    }
}
void cleanup_failure() {
    Fixture f; f.run(); const auto t = f.reserve();
    f.backend.cleanup = {E_ACCESSDENIED, E_FAIL, false, false};
    check(f.session.stop() == E_ACCESSDENIED, "stop reports first error");
    check(f.session.status().cleanup.reset_error == E_FAIL, "reset error retained");
    check(FAILED(f.session.commit(t, Source::self_microphone, f.voice.data(), 480)), "old ticket revoked even when cleanup fails");
    check(FAILED(f.session.prepare(inventory(), pair, 2, 2)), "no restart after cleanup error");
    f.backend.cleanup = {S_OK, S_OK, true, false};
    check(f.session.stop() == S_OK && f.session.status().cleanup.client_reset, "explicit cleanup retry");
    check(!f.session.status().cleanup.external_drain_verified && FAILED(f.session.prepare(inventory(), pair, 2, 2)),
          "client reset does not establish downstream drain");
}
void restarts_fake_only() {
    Fixture f;
    Ticket old;
    for (uint64_t g = 1; g <= 4; ++g) {
        f.run(g);
        if (g > 1) check(FAILED(f.session.commit(old, Source::self_microphone, f.voice.data(), 480)), "old generation");
        old = f.reserve(g);
        check(f.session.commit(old, Source::self_microphone, f.voice.data(), 480) == S_OK, "new voice");
        check(f.session.stop() == S_OK && f.backend.pcm.empty(), "drain in-memory queue");
        const auto stops = f.backend.stops;
        check(f.session.stop() == S_OK && f.backend.stops == stops, "idempotent stop");
    }
}
void owner_thread() {
    Fixture f; f.run(); HRESULT hr = S_OK;
    std::thread other([&] { hr = f.session.stop(); }); other.join();
    check(hr == RPC_E_WRONG_THREAD && f.session.status().phase == Phase::running, "MTA owner affinity");
}
void conversion() {
    float source[] = {-1, -.25f, 0, .5f, 1};
    float output[10] = {};
    check(copy_mono_interleaved(source, 5, 1, output, 10) == S_OK, "mono");
    check(std::equal(source, source + 5, output), "mono unchanged");
    check(copy_mono_interleaved(source, 5, 2, output, 9) == E_INVALIDARG, "buffer bounds");
    source[4] = std::numeric_limits<float>::infinity();
    std::fill(std::begin(output), std::end(output), -.75f);
    check(copy_mono_interleaved(source, 5, 2, output, 10) == E_INVALIDARG, "finite");
    check(std::all_of(std::begin(output), std::end(output), [](float s){ return s == -.75f; }), "no partial invalid write");
}
}
int main() {
    struct Test { const char* name; void(*run)(); };
    const Test tests[] = {{"endpoint identity", identity}, {"default off", default_off}, {"format bounds", formats},
        {"PCM and device clock", pcm_and_clock}, {"backpressure", backpressure}, {"source and payload", wrong_source_or_pcm},
        {"late and invalid clocks", late_underrun_clock}, {"backend failures", failures}, {"cleanup errors", cleanup_failure},
        {"three synthetic restarts", restarts_fake_only}, {"owner thread", owner_thread}, {"sample conversion", conversion}};
    int failed = 0;
    for (const auto& test : tests) try { test.run(); std::cout << "PASS " << test.name << '\n'; }
        catch (const std::exception& e) { ++failed; std::cerr << "FAIL " << test.name << ": " << e.what() << '\n'; }
    std::cout << "cases=" << std::size(tests) << " failed=" << failed << " media_streams=0\n";
    return failed ? 1 : 0;
}
