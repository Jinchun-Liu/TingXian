#include "audio_fixture.h"
#include <array>
#include <algorithm>
using namespace tingxian::audio;
namespace {
class Fixture final : public Backend {
public:
    HANDLE ready = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    bool initialized = false;
    ~Fixture() override { if (ready) CloseHandle(ready); }
    HANDLE ready_event() const noexcept override { return initialized ? ready : nullptr; }
    Format format{48000, 2, 1920, 10000000};
    Snapshot now{0, 0, 100000000};
    std::array<float, 1920> last{};
    UINT32 count = 0, failure = 0;
    HRESULT prepare(const Pair&, UINT32 channels, Format& out) noexcept override {
        if (!ready) return E_OUTOFMEMORY;
        if (failure == 7) return E_FAIL; // Failure after prepare may have acquired resources.
        ResetEvent(ready); initialized = true;
        format.channels = channels; out = format; return S_OK;
    }
    HRESULT start() noexcept override {
        if (now.padding != 960) return E_UNEXPECTED;
        SetEvent(ready); return S_OK;
    }
    HRESULT snapshot(Snapshot& out) noexcept override {
        if (failure == 1) return HRESULT_FROM_WIN32(ERROR_DEVICE_NOT_CONNECTED);
        out = now; return S_OK;
    }
    HRESULT write_mono(const float* mono, UINT32 frames, bool silence) noexcept override {
        if (failure == 2) return E_FAIL;
        if (!frames || frames > 960 || now.padding > 1920 || frames > 1920 - now.padding) return E_INVALIDARG;
        last.fill(0);
        if (!silence) {
            auto hr = copy_mono_interleaved(mono, frames, format.channels, last.data(), UINT32(last.size()));
            if (FAILED(hr)) return hr;
        }
        count = frames * format.channels;
        now.padding += frames;
        return S_OK;
    }
    Cleanup stop_reset() noexcept override {
        Cleanup c{failure == 3 ? E_ACCESSDENIED : S_OK, failure == 4 ? E_FAIL : S_OK, false, false};
        c.client_reset = SUCCEEDED(c.stop_error) && SUCCEEDED(c.reset_error);
        c.external_drain_verified = c.client_reset && failure != 5;
        if (c.client_reset) { last.fill(0); count = 0; now = {}; }
        if (ready) ResetEvent(ready);
        initialized = false;
        return c;
    }
};
}
std::unique_ptr<Backend> make_fixture_backend() { return std::make_unique<Fixture>(); }
std::vector<Endpoint> fixture_inventory(const Backend& backend) {
    const auto& f = static_cast<const Fixture&>(backend);
    return {{L"fixture-render", L"Synthetic", L"fixture-adapter", L"VBAudioVACWDM", L"VBAudioVACMME", Flow::render, f.failure != 6},
            {L"fixture-capture", L"Synthetic", L"fixture-adapter", L"VBAudioVACWDM", L"VBAudioVACMME", Flow::capture, true}};
}
HRESULT fixture_step(Backend& backend, UINT32 consumed, UINT32 failure) noexcept {
    auto& f = static_cast<Fixture&>(backend);
    if (consumed > 4800 || failure > 7) return E_INVALIDARG;
    const auto ticks = UINT64(consumed) * 10000000 / 48000;
    f.now.position += ticks; f.now.qpc_100ns += ticks;
    f.now.padding -= std::min(f.now.padding, consumed);
    f.failure = failure;
    // Mirror production event-driven readiness when the synthetic device clock
    // advances. A missing event made host tests depend on a 5ms polling timeout
    // despite their 18ms bootstrap lease; do not relax the production deadline.
    if (consumed || failure) SetEvent(f.ready);
    return S_OK;
}
HRESULT fixture_read(Backend& backend, float* out, UINT32 capacity, UINT32* samples) noexcept {
    if (!samples) return E_POINTER;
    *samples = 0;
    const auto& f = static_cast<Fixture&>(backend);
    if (capacity < f.count || (!out && f.count)) return E_INVALIDARG;
    if (f.count) std::copy_n(f.last.data(), f.count, out);
    *samples = f.count;
    return S_OK;
}
HRESULT fixture_anchor(Backend& backend, UINT64 qpc) noexcept {
    auto& f = static_cast<Fixture&>(backend);
    if (!qpc || f.now.position || f.now.padding || f.count) return E_INVALIDARG;
    f.now.qpc_100ns = qpc;
    return S_OK;
}
