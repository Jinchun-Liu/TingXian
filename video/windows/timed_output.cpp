#include "timed_output.h"
#include <limits>
namespace tingxian::video {
namespace {
constexpr HRESULT Cancelled = HRESULT_FROM_WIN32(ERROR_CANCELLED);
std::atomic<UINT64> next_identity{1};
UINT64 identity() noexcept {
    auto value=next_identity.load();
    while(value!=(std::numeric_limits<UINT64>::max)()) {
        if(next_identity.compare_exchange_weak(value,value+1))return value;
    }
    return 0;
}
}
TimedPipeOutput::TimedPipeOutput() : identity_(identity()) { wake_ = CreateEventW(nullptr, FALSE, FALSE, nullptr); }
TimedPipeOutput::~TimedPipeOutput() { stop(); if(wake_) CloseHandle(wake_); }
HRESULT TimedPipeOutput::open(UINT64 generation, VideoPipeEndpoint* endpoint) noexcept {
    if (!endpoint) return E_POINTER;
    SecureZeroMemory(endpoint, sizeof(*endpoint));
    std::lock_guard guard(mutex_);
    if (!wake_) return E_OUTOFMEMORY;
    if (!identity_) return E_UNEXPECTED;
    if (revoked_ || status_.phase != TimedOutputPhase::idle || sender_.joinable()) return Cancelled;
    if (!generation) return E_INVALIDARG;
    status_.generation = generation; status_.local_clear = 0;
    HRESULT hr = E_OUTOFMEMORY;
    try {
        hr = publisher_.open_timed(endpoint);
        if (SUCCEEDED(hr)) {
            status_.phase = TimedOutputPhase::waiting;
            sender_ = std::thread(&TimedPipeOutput::run, this);
            return S_OK;
        }
    } catch (...) { hr = E_OUTOFMEMORY; }
    publisher_.close(); SecureZeroMemory(endpoint, sizeof(*endpoint));
    status_.phase = TimedOutputPhase::fault; status_.error = hr; status_.local_clear = 1;
    revoked_ = true; notify(); return hr;
}
HRESULT TimedPipeOutput::reserve(UINT64 generation, TimedTicket* ticket) noexcept {
    if (!ticket) return E_POINTER;
    *ticket = {};
    std::lock_guard guard(mutex_);
    if (revoked_ || generation != status_.generation || !generation) return Cancelled;
    if (status_.phase == TimedOutputPhase::waiting || status_.pending) return S_FALSE;
    if (status_.phase != TimedOutputPhase::ready) return E_UNEXPECTED;
    if (status_.serial == (std::numeric_limits<UINT64>::max)()) return E_UNEXPECTED;
    ++status_.serial; status_.pending = 1; status_.phase = TimedOutputPhase::reserved;
    *ticket = {identity_, generation, status_.serial}; return S_OK;
}
HRESULT TimedPipeOutput::stage(TimedTicket ticket, const TimedFrame& timing, const BYTE* bytes, UINT32 size) noexcept {
    std::lock_guard guard(mutex_);
    UINT64 now = 0;
    if (revoked_ || !owns(ticket) || status_.phase != TimedOutputPhase::reserved) return Cancelled;
    if (!qpc_now(now) || !valid_timing(timing, now) ||
        (timing.flags == TimedNeutral ? (bytes || size) : (!bytes || size != FrameBytes))) {
        ++status_.rejected; return E_INVALIDARG;
    }
    try {
        auto next = std::make_unique<Pending>(); next->timing = timing;
        if(size) next->pixels.assign(bytes, bytes + size);
        pending_ = std::move(next);
    } catch (...) { ++status_.rejected; return E_OUTOFMEMORY; }
    ++status_.submitted; status_.phase = TimedOutputPhase::sending;
    changed_.notify_all(); return S_OK; // bounded copy only, no pipe I/O here
}
HRESULT TimedPipeOutput::take(TimedTicket ticket, TimedReceipt* receipt) noexcept {
    if(!receipt) return E_POINTER;
    *receipt = {};
    std::lock_guard guard(mutex_);
    if(!owns(ticket) || !status_.pending) return E_INVALIDARG;
    if(!receipt_) return S_FALSE;
    *receipt = *receipt_; receipt_.reset(); status_.pending = 0;
    if(status_.phase == TimedOutputPhase::receipt) status_.phase = TimedOutputPhase::ready;
    return S_OK;
}
void TimedPipeOutput::cancel() noexcept {
    std::lock_guard guard(mutex_);
    if(revoked_) return;
    revoked_ = true; pending_.reset();
    if(status_.pending) {
        // Cancellation supersedes a late admission. It does not retract bytes
        // that a consumer may already have received before cancellation.
        receipt_ = TimedReceipt{2,Admission::cancelled,status_.generation,status_.serial,Cancelled,0};
        ++status_.cancelled;
    }
    if(status_.phase != TimedOutputPhase::fault) status_.phase = TimedOutputPhase::stopping;
    publisher_.cancel(); notify();
}
void TimedPipeOutput::stop() noexcept {
    cancel();
    if(sender_.joinable()) sender_.join();
    publisher_.close();
    std::lock_guard guard(mutex_);
    pending_.reset(); status_.local_clear = 1;
    if(status_.phase != TimedOutputPhase::fault) status_.phase = TimedOutputPhase::stopped;
    notify();
}
TimedOutputStatus TimedPipeOutput::status() const noexcept { std::lock_guard guard(mutex_); return status_; }
void TimedPipeOutput::run() noexcept {
    HRESULT hr = E_UNEXPECTED;
    try {
        hr = publisher_.accept();
        if(SUCCEEDED(hr)) hr = publisher_.begin(status_.generation);
        {
            std::lock_guard guard(mutex_);
            if(SUCCEEDED(hr) && !revoked_) { status_.phase = TimedOutputPhase::ready; notify(); }
        }
        while(SUCCEEDED(hr)) {
            std::unique_ptr<Pending> current;
            UINT64 generation=0, serial=0;
            {
                std::unique_lock guard(mutex_);
                changed_.wait(guard,[&]{return revoked_ || bool(pending_);});
                if(revoked_) break;
                current = std::move(pending_); generation=status_.generation; serial=status_.serial;
            }
            bool received=false;
            hr=publisher_.submit_timed(generation,serial,current->timing,
                current->pixels.empty()?nullptr:current->pixels.data(),UINT32(current->pixels.size()),&received);
            current.reset();
            {
                std::lock_guard guard(mutex_);
                if(revoked_) break;
                const auto outcome=received ? (SUCCEEDED(hr)?Admission::admitted:Admission::rejected) : Admission::unknown;
                receipt_=TimedReceipt{2,outcome,generation,serial,hr,0};
                if(outcome==Admission::admitted) {++status_.admitted;status_.phase=TimedOutputPhase::receipt;}
                else {++status_.rejected;status_.error=hr;status_.phase=TimedOutputPhase::fault;revoked_=true;}
                notify();
            }
        }
    } catch (...) { hr=E_OUTOFMEMORY; }
    publisher_.close();
    std::lock_guard guard(mutex_);
    pending_.reset(); status_.local_clear=1;
    if(!revoked_) {
        revoked_=true;status_.phase=TimedOutputPhase::fault;status_.error=FAILED(hr)?hr:E_UNEXPECTED;
        if(status_.pending && !receipt_) receipt_=TimedReceipt{2,Admission::unknown,status_.generation,status_.serial,status_.error,0};
    } else if(status_.phase!=TimedOutputPhase::fault) status_.phase=TimedOutputPhase::stopped;
    notify();
}
}
