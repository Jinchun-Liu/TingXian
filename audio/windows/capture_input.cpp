#include "capture_input.h"
#include <audioclient.h>
#include <mmdeviceapi.h>
#include <algorithm>
#include <cmath>

namespace tingxian::capture {
HRESULT validate_selection(const std::vector<audio::Endpoint>& endpoints, const Selection& selected) noexcept {
    if (selected.microphone_id.empty() || selected.output_return_id.empty() ||
        selected.microphone_id.size() > 1024 || selected.output_return_id.size() > 1024 ||
        selected.microphone_id.find(L'\0') != std::wstring::npos || selected.output_return_id.find(L'\0') != std::wstring::npos ||
        selected.microphone_id == selected.output_return_id) return E_INVALIDARG;
    const audio::Endpoint* found = nullptr;
    for (const auto& e : endpoints) if (e.id == selected.microphone_id) {
        if (found) return E_UNEXPECTED;
        found = &e;
    }
    if (!found || !found->active) return HRESULT_FROM_WIN32(ERROR_DEVICE_NOT_CONNECTED);
    if (found->flow != audio::Flow::capture || audio::is_basic_cable(*found) ||
        (found->form_factor != Microphone && found->form_factor != Headset && found->form_factor != Handset))
        return E_ACCESSDENIED;
    return S_OK;
}
HRESULT Session::fault(HRESULT error) noexcept {
    if (status_.operation_error == S_OK) status_.operation_error = FAILED(error) ? error : E_UNEXPECTED;
    stop();
    status_.phase = Phase::fault;
    return status_.operation_error;
}
Session::~Session() { if (GetCurrentThreadId() == owner_) stop(); }
HRESULT Session::prepare(const Selection& selected, uint64_t generation, bool intent) noexcept {
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (!intent) return E_ACCESSDENIED;
    if (!generation || status_.phase != Phase::idle) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    auto hr = backend_.inspect(selected);
    if (hr != S_OK) return FAILED(hr) ? hr : E_UNEXPECTED;
    generation_ = generation;
    resources_ = true; // Failed prepare may already own an initialized client.
    hr = backend_.prepare(selected, format_);
    if (hr != S_OK) return fault(hr);
    if (format_.rate != rate || format_.channels != 1 || !format_.buffer_frames || format_.buffer_frames > max_frames)
        return fault(AUDCLNT_E_UNSUPPORTED_FORMAT);
    status_.phase = Phase::prepared;
    return S_OK;
}
HRESULT Session::start(uint64_t generation) noexcept {
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (generation != generation_ || status_.phase != Phase::prepared) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    const auto hr = backend_.start();
    if (hr != S_OK) return fault(hr);
    status_.phase = Phase::running;
    return S_OK;
}
HRESULT Session::read(uint64_t generation, Packet& out) noexcept {
    out.clear();
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (generation != generation_ || status_.phase != Phase::running) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    RawPacket raw;
    auto hr = backend_.acquire(raw);
    if (hr == S_FALSE) return S_FALSE;
    if (hr != S_OK) return fault(hr);
    if (!raw.frames) return fault(E_UNEXPECTED);
    uint64_t now = 0;
    hr = backend_.now(now);
    const auto bad_clock = HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
    if (hr == S_OK && (raw.frames > max_frames || raw.frames > format_.buffer_frames ||
        raw.first_frame > (std::numeric_limits<uint64_t>::max)() - raw.frames)) hr = E_INVALIDARG;
    if (hr == S_OK && ((raw.flags & ~(AUDCLNT_BUFFERFLAGS_SILENT | AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY |
        AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR)) || (raw.flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR) ||
        (have_packet_ && (raw.flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY)))) hr = bad_clock;
    if (hr == S_OK && (!raw.qpc_100ns || raw.qpc_100ns > now || now - raw.qpc_100ns > max_age ||
        uint64_t(raw.frames - 1) * 10000000 / rate > now - raw.qpc_100ns)) hr = bad_clock;
    if (hr == S_OK && have_packet_) {
        const auto nominal = uint64_t(last_frames_) * 10000000 / rate;
        if (raw.first_frame != next_frame_ || raw.qpc_100ns <= last_qpc_) hr = bad_clock;
        else {
            const auto delta = raw.qpc_100ns - last_qpc_;
            if ((delta > nominal ? delta - nominal : nominal - delta) > clock_residual) hr = bad_clock;
        }
    }
    if (hr == S_OK && !(raw.flags & AUDCLNT_BUFFERFLAGS_SILENT)) {
        if (!raw.samples) hr = E_POINTER;
        else for (uint32_t i = 0; i < raw.frames; ++i) {
            if (!std::isfinite(raw.samples[i])) { hr = E_INVALIDARG; break; }
            out.samples[i] = raw.samples[i];
        }
    }
    // Release every acquired nonempty packet exactly once BEFORE exposing PCM.
    const auto release = backend_.release(raw.frames);
    if (release != S_OK) status_.release_error = FAILED(release) ? release : E_UNEXPECTED;
    if (hr != S_OK || release != S_OK) {
        status_.discarded_frames += raw.frames;
        out.clear();
        return fault(hr != S_OK ? hr : status_.release_error);
    }
    out.generation = generation_;
    out.first_frame = raw.first_frame;
    out.qpc_100ns = raw.qpc_100ns;
    out.frames = raw.frames;
    out.flags = raw.flags;
    // There is no predecessor before the first packet of a new session. Retain
    // its original flag and count this boundary; NEVER forgive a later gap.
    if (!have_packet_ && (raw.flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY)) ++status_.startup_discontinuities;
    next_frame_ = raw.first_frame + raw.frames;
    last_frames_ = raw.frames;
    last_qpc_ = raw.qpc_100ns;
    have_packet_ = true;
    status_.delivered_frames += raw.frames;
    ++status_.packets;
    return S_OK;
}
HRESULT Session::stop() noexcept {
    if (GetCurrentThreadId() != owner_) return RPC_E_WRONG_THREAD;
    if (resources_) {
        status_.cleanup = backend_.stop_reset();
        resources_ = false;
    }
    HRESULT result = S_OK;
    if (!cleanup_success(status_.cleanup.stop_error)) result = FAILED(status_.cleanup.stop_error) ? status_.cleanup.stop_error : E_UNEXPECTED;
    else if (!cleanup_success(status_.cleanup.reset_error)) result = FAILED(status_.cleanup.reset_error) ? status_.cleanup.reset_error : E_UNEXPECTED;
    else if (generation_ && !status_.cleanup.reset) result = E_UNEXPECTED;
    if (FAILED(result)) {
        if (status_.operation_error == S_OK) status_.operation_error = result;
        status_.phase = Phase::fault;
    } else if (status_.phase != Phase::fault) status_.phase = Phase::stopped;
    return result;
}
}
