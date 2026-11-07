#include "audio_output.h"
#include <cmath>
#include <limits>

namespace tingxian::audio {
namespace {
bool equal(const std::wstring& a, const wchar_t* b) noexcept {
    return CompareStringOrdinal(a.c_str(), -1, b, -1, TRUE) == CSTR_EQUAL;
}
constexpr HRESULT state_error = HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
constexpr HRESULT stale_error = HRESULT_FROM_WIN32(ERROR_REVISION_MISMATCH);
constexpr uint64_t deadline_guard = 96; // 2 ms admission guard, not a real-time guarantee.
}
bool is_basic_cable(const Endpoint& e) noexcept {
    return !e.adapter_id.empty() && equal(e.hardware_id, L"VBAudioVACWDM") &&
        equal(e.service, L"VBAudioVACMME");
}
HRESULT copy_mono_interleaved(const float* mono, uint32_t frames, uint32_t channels,
                              float* output, uint32_t output_samples) noexcept {
    if (!mono || !output || !frames || frames > max_write_frames || (channels != 1 && channels != 2) ||
        output_samples < frames * channels) return E_INVALIDARG;
    for (uint32_t i = 0; i < frames; ++i)
        if (!std::isfinite(mono[i]) || std::abs(mono[i]) > 1.0f) return E_INVALIDARG;
    for (uint32_t i = 0; i < frames; ++i)
        for (uint32_t ch = 0; ch < channels; ++ch) output[i * channels + ch] = mono[i];
    return S_OK;
}
HRESULT validate_pair(const std::vector<Endpoint>& endpoints, const Pair& p) noexcept {
    if (p.render_id.empty() || p.capture_id.empty() || p.render_id == p.capture_id) return E_INVALIDARG;
    const Endpoint *r = nullptr, *c = nullptr;
    for (const auto& e : endpoints) {
        if (e.id == p.render_id) { if (r) return E_INVALIDARG; r = &e; }
        if (e.id == p.capture_id) { if (c) return E_INVALIDARG; c = &e; }
    }
    if (!r || !c || !r->active || !c->active || r->flow != Flow::render ||
        c->flow != Flow::capture || !is_basic_cable(*r) || !is_basic_cable(*c) ||
        !equal(r->adapter_id, c->adapter_id.c_str())) return E_ACCESSDENIED;
    return S_OK;
}
Session::~Session() { stop(); }
HRESULT Session::fault(HRESULT error) noexcept {
    lease_.revoke();
    pending_ = {};
    status_.phase = Phase::fault;
    status_.operation_error = error;
    if (resources_) status_.cleanup = backend_.stop_reset();
    return error;
}
HRESULT Session::prepare(const std::vector<Endpoint>& inventory, const Pair& pair,
                         uint32_t channels, uint64_t generation) noexcept {
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (status_.phase != Phase::idle && status_.phase != Phase::stopped) return state_error;
    if (resources_ && (!status_.cleanup.client_reset || !status_.cleanup.external_drain_verified))
        return state_error;
    if (!generation || generation <= generation_ || (channels != 1 && channels != 2)) return E_INVALIDARG;
    HRESULT hr = validate_pair(inventory, pair);
    if (FAILED(hr)) return hr;
    resources_ = true; // Even failed preparation may have acquired resources.
    status_ = {};
    format_ = {};
    hr = backend_.prepare(pair, channels, format_);
    if (FAILED(hr)) return fault(hr);
    if (format_.rate != sample_rate || format_.channels != channels ||
        format_.buffer_frames < max_write_frames || format_.buffer_frames > max_buffer_frames ||
        !format_.clock_frequency || format_.clock_frequency > 1000000000ULL)
        return fault(HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED));
    generation_ = generation;
    pending_ = {};
    have_clock_ = false;
    last_clock_ = {};
    status_.phase = Phase::prepared;
    return S_OK;
}
HRESULT Session::start(uint64_t generation, bool intent) noexcept {
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (generation != generation_) return stale_error;
    if (status_.phase != Phase::prepared) return state_error;
    if (!intent) return E_ACCESSDENIED;
    // Prime only silence; this is never an excuse to reuse earlier voice.
    HRESULT hr = backend_.write_mono(nullptr, max_write_frames, true);
    if (FAILED(hr)) return fault(hr);
    status_.submitted_frames = max_write_frames;
    hr = backend_.start();
    if (FAILED(hr)) return fault(hr);
    status_.phase = Phase::running;
    return S_OK;
}
HRESULT Session::observe(Snapshot& s, uint64_t& consumed) noexcept {
    HRESULT hr = backend_.snapshot(s);
    if (hr != S_OK) return fault(FAILED(hr) ? hr : HRESULT_FROM_WIN32(ERROR_RETRY));
    if (s.padding > format_.buffer_frames || !s.qpc_100ns ||
        (have_clock_ && (s.position < last_clock_.position || s.qpc_100ns < last_clock_.qpc_100ns)))
        return fault(HRESULT_FROM_WIN32(ERROR_INVALID_DATA));
    const auto frequency = format_.clock_frequency;
    const auto whole = s.position / frequency;
    if (whole > (std::numeric_limits<uint64_t>::max() - sample_rate) / sample_rate)
        return fault(HRESULT_FROM_WIN32(ERROR_ARITHMETIC_OVERFLOW));
    // Round consumed position upward. Frequency is bounded, remainder product is safe.
    consumed = whole * sample_rate + ((s.position % frequency) * sample_rate + frequency - 1) / frequency;
    if (consumed > status_.submitted_frames) return fault(HRESULT_FROM_WIN32(ERROR_NO_DATA));
    last_clock_ = s;
    have_clock_ = true;
    return S_OK;
}
HRESULT Session::reserve(uint64_t generation, uint32_t frames, Ticket& ticket) noexcept {
    ticket = {};
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (generation != generation_) return stale_error;
    if (status_.phase != Phase::running || pending_.serial) return state_error;
    if (!frames || frames > max_write_frames) return E_INVALIDARG;
    Snapshot s;
    uint64_t consumed = 0;
    HRESULT hr = observe(s, consumed);
    if (FAILED(hr)) return hr;
    if (status_.submitted_frames - consumed < deadline_guard) return fault(HRESULT_FROM_WIN32(ERROR_TIMEOUT));
    if (frames > format_.buffer_frames - s.padding) return S_FALSE;
    if (serial_ == std::numeric_limits<uint64_t>::max() ||
        status_.submitted_frames > std::numeric_limits<uint64_t>::max() - frames)
        return fault(HRESULT_FROM_WIN32(ERROR_ARITHMETIC_OVERFLOW));
    pending_ = {generation_, ++serial_, status_.submitted_frames, frames, s};
    ticket = pending_;
    return S_OK;
}
HRESULT Session::commit(const Ticket& t, Source source, const float* mono, uint32_t frames) noexcept {
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (t.generation != generation_ || !t.serial || t.serial != pending_.serial) return stale_error;
    if (status_.phase != Phase::running) return state_error;
    if (t.first_frame != pending_.first_frame || t.frames != pending_.frames || frames != t.frames ||
        t.clock.position != pending_.clock.position || t.clock.qpc_100ns != pending_.clock.qpc_100ns ||
        t.clock.padding != pending_.clock.padding ||
        source != Source::self_microphone || !mono) return fault(E_INVALIDARG);
    for (uint32_t i = 0; i < frames; ++i)
        if (!std::isfinite(mono[i]) || std::abs(mono[i]) > 1.0f) return fault(E_INVALIDARG);
    Snapshot s;
    uint64_t consumed = 0;
    HRESULT hr = observe(s, consumed);
    if (FAILED(hr)) return hr;
    if (t.first_frame - consumed < deadline_guard || s.qpc_100ns - pending_.clock.qpc_100ns > 200000)
        return fault(HRESULT_FROM_WIN32(ERROR_TIMEOUT));
    if (frames > format_.buffer_frames - s.padding) return fault(HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW));
    hr = backend_.write_mono(mono, frames, false);
    pending_ = {};
    if (FAILED(hr)) return fault(hr);
    status_.submitted_frames += frames;
    return S_OK;
}
HRESULT Session::stop() noexcept {
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    lease_.revoke();
    pending_ = {};
    if (resources_ && !status_.cleanup.client_reset) status_.cleanup = backend_.stop_reset();
    status_.phase = Phase::stopped;
    if (FAILED(status_.cleanup.stop_error)) return status_.cleanup.stop_error;
    if (FAILED(status_.cleanup.reset_error)) return status_.cleanup.reset_error;
    return S_OK;
}
}
