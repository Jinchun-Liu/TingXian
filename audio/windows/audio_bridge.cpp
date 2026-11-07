#include "audio_bridge.h"
#include "audio_output.h"
#include <algorithm>
#include <atomic>
#include <new>
#ifdef TINGXIAN_AUDIO_FIXTURE
#include "audio_fixture.h"
#endif
using namespace tingxian::audio;
static_assert(sizeof(TxAudioFormat) == 24);
static_assert(sizeof(TxAudioClock) == 24);
static_assert(sizeof(TxAudioTicket) == 56);
static_assert(sizeof(TxAudioStatus) == 48);
namespace {
struct Lease final : OutputLease {
    TxAudioRevoke callback;
    void* context;
    bool revoked = false;
    Lease(TxAudioRevoke fn, void* ctx) : callback(fn), context(ctx) {}
    void revoke() noexcept override {
        if (!revoked) { revoked = true; callback(context); }
    }
};
struct Owner {
    const DWORD thread = GetCurrentThreadId();
    std::unique_ptr<Backend> backend;
    Lease lease;
    Session session;
    bool attempted = false;
    Owner(TxAudioRevoke fn, void* ctx) :
#ifdef TINGXIAN_AUDIO_FIXTURE
        backend(make_fixture_backend()),
#else
        backend(make_windows_backend()),
#endif
        lease(fn, ctx), session(*backend, lease) {}
};
Owner* owner(void* raw) noexcept { return static_cast<Owner*>(raw); }
HRESULT valid(void* raw) noexcept {
    return !raw ? E_POINTER : owner(raw)->thread != GetCurrentThreadId() ? RPC_E_WRONG_THREAD : S_OK;
}
bool valid_id(const wchar_t* id, UINT32 length) noexcept {
    return id && length && length <= 1024 && std::find(id, id + length, L'\0') == id + length;
}
TxAudioClock clock_out(const Snapshot& s) noexcept { return {s.position, s.qpc_100ns, s.padding, 0}; }
}
extern "C" {
UINT32 WINAPI TxAudioAbi() noexcept { return 1; }
void* WINAPI TxAudioCreate(TxAudioRevoke callback, void* context) noexcept {
    if (!callback || !context) return nullptr;
    try { return new Owner(callback, context); } catch (...) { return nullptr; }
}
HRESULT WINAPI TxAudioPrepare(void* raw, const wchar_t* render, UINT32 rlen, const wchar_t* capture,
    UINT32 clen, UINT32 channels, UINT64 generation, TxAudioFormat* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (!valid_id(render, rlen) || !valid_id(capture, clen) || !generation || (channels != 1 && channels != 2)) return E_INVALIDARG;
    auto& o = *owner(raw);
    // Single session object: successful cleanup is not permission to reopen a
    // downstream bridge. Host must retain any uncertain drain across recreation.
    if (o.attempted) return HRESULT_FROM_WIN32(ERROR_INVALID_STATE);
    o.attempted = true;
    try {
        std::vector<Endpoint> endpoints;
#ifdef TINGXIAN_AUDIO_FIXTURE
        endpoints = fixture_inventory();
#else
        hr = enumerate_endpoints(endpoints); if (FAILED(hr)) return hr;
#endif
        hr = o.session.prepare(endpoints, {std::wstring(render, rlen), std::wstring(capture, clen)}, channels, generation);
        if (hr != S_OK) return hr;
        const auto f = o.session.format();
        *out = {1, f.rate, f.channels, f.buffer_frames, f.clock_frequency};
        return S_OK;
    } catch (const std::bad_alloc&) { o.session.stop(); return E_OUTOFMEMORY; }
    catch (...) { o.session.stop(); return E_FAIL; }
}
HRESULT WINAPI TxAudioStart(void* raw, UINT64 generation, UINT32 intent) noexcept {
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (intent > 1) return E_INVALIDARG;
    return owner(raw)->session.start(generation, intent == 1);
}
HRESULT WINAPI TxAudioObserve(void* raw, UINT64 generation, TxAudioClock* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    auto hr = valid(raw); if (FAILED(hr)) return hr;
    Snapshot s;
    hr = owner(raw)->session.clock_snapshot(generation, s);
    if (hr == S_OK) *out = clock_out(s);
    return hr;
}
HRESULT WINAPI TxAudioReserve(void* raw, UINT64 generation, UINT32 frames, TxAudioTicket* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    auto hr = valid(raw); if (FAILED(hr)) return hr;
    Ticket t;
    hr = owner(raw)->session.reserve(generation, frames, t);
    if (hr == S_OK) *out = {t.generation, t.serial, t.first_frame, t.frames, 0, clock_out(t.clock)};
    return hr; // S_FALSE stays backpressure, never an accepted ticket.
}
HRESULT WINAPI TxAudioCommit(void* raw, const TxAudioTicket* t, UINT32 size, UINT32 source,
    const float* mono, UINT32 frames) noexcept {
    auto hr = valid(raw); if (FAILED(hr)) return hr;
    if (!t || size != sizeof(*t) || t->reserved || t->clock.reserved || source > UINT32(Source::output_return)) return E_INVALIDARG;
    const Ticket ticket{t->generation, t->serial, t->first_frame, t->frames,
                         {t->clock.padding, t->clock.position, t->clock.qpc_100ns}};
    return owner(raw)->session.commit(ticket, Source(source), mono, frames);
}
HRESULT WINAPI TxAudioStop(void* raw) noexcept {
    const auto hr = valid(raw); return FAILED(hr) ? hr : owner(raw)->session.stop();
}
HRESULT WINAPI TxAudioGetStatus(void* raw, TxAudioStatus* out, UINT32 size) noexcept {
    if (!out || size != sizeof(*out)) return E_INVALIDARG;
    *out = {};
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    const auto& o = *owner(raw);
    const auto s = o.session.status();
    *out = {1, UINT32(s.phase), s.submitted_frames, s.operation_error, s.cleanup.stop_error,
        s.cleanup.reset_error, s.cleanup.client_reset ? 1u : 0u, s.cleanup.external_drain_verified ? 1u : 0u,
        o.lease.revoked ? 1u : 0u, {0, 0}};
    return S_OK;
}
HRESULT WINAPI TxAudioDestroy(void* raw) noexcept {
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    delete owner(raw); return S_OK;
}
HRESULT WINAPI TxAudioReadyEvent(void* raw, HANDLE* event) noexcept {
    if (!event) return E_POINTER;
    *event = nullptr;
    const auto hr = valid(raw); if (FAILED(hr)) return hr;
    *event = owner(raw)->backend->ready_event();
    return S_OK; // Borrowed only, never CloseHandle; lifetime is this owner.
}
#ifdef TINGXIAN_AUDIO_FIXTURE
// Test-only control exports; absent from the production DLL's build and .def.
__declspec(dllexport) HRESULT WINAPI TxAudioFixtureStep(void* raw, UINT32 consumed, UINT32 failure) noexcept {
    const auto hr = valid(raw); return FAILED(hr) ? hr : fixture_step(*owner(raw)->backend, consumed, failure);
}
__declspec(dllexport) HRESULT WINAPI TxAudioFixtureRead(void* raw, float* out, UINT32 capacity, UINT32* samples) noexcept {
    const auto hr = valid(raw); return FAILED(hr) ? hr : fixture_read(*owner(raw)->backend, out, capacity, samples);
}
#endif
}
