#include "timed_bridge.h"
#include <cstring>
using namespace tingxian::video;
namespace {
struct Instance { DWORD thread=GetCurrentThreadId(); TimedPipeOutput output; };
HRESULT owner(void* raw) noexcept {
    return !raw ? E_INVALIDARG : static_cast<Instance*>(raw)->thread==GetCurrentThreadId() ? S_OK : RPC_E_WRONG_THREAD;
}
}
extern "C" {
UINT32 WINAPI TxTimedAbi() noexcept { return 2; }
void* WINAPI TxTimedCreate() noexcept { try { return new Instance; } catch(...) { return nullptr; } }
HRESULT WINAPI TxTimedOpen(void* raw,UINT64 generation,BYTE* bytes,UINT32 length) noexcept {
    const auto hr=owner(raw);if(FAILED(hr))return hr;
    if(!bytes || length!=sizeof(VideoPipeEndpoint))return E_INVALIDARG;
    SecureZeroMemory(bytes,length);VideoPipeEndpoint endpoint{};
    const auto result=static_cast<Instance*>(raw)->output.open(generation,&endpoint);
    if(SUCCEEDED(result))std::memcpy(bytes,&endpoint,length);
    SecureZeroMemory(&endpoint,sizeof(endpoint));return result;
}
HRESULT WINAPI TxTimedReserve(void* raw,UINT64 generation,TimedTicket* ticket,UINT32 length) noexcept {
    const auto hr=owner(raw);if(FAILED(hr))return hr;
    if(!ticket || length!=sizeof(*ticket))return E_INVALIDARG;
    return static_cast<Instance*>(raw)->output.reserve(generation,ticket);
}
HRESULT WINAPI TxTimedStage(void* raw,const TimedTicket* ticket,UINT32 ts,const TimedFrame* frame,UINT32 fs,
    const BYTE* bytes,UINT32 length) noexcept {
    const auto hr=owner(raw);if(FAILED(hr))return hr;
    if(!ticket || ts!=sizeof(*ticket) || !frame || fs!=sizeof(*frame))return E_INVALIDARG;
    return static_cast<Instance*>(raw)->output.stage(*ticket,*frame,bytes,length);
}
HRESULT WINAPI TxTimedTake(void* raw,const TimedTicket* ticket,UINT32 ts,TimedReceipt* receipt,UINT32 rs) noexcept {
    const auto hr=owner(raw);if(FAILED(hr))return hr;
    if(!ticket || ts!=sizeof(*ticket) || !receipt || rs!=sizeof(*receipt))return E_INVALIDARG;
    return static_cast<Instance*>(raw)->output.take(*ticket,receipt);
}
HRESULT WINAPI TxTimedStatus(void* raw,TimedOutputStatus* status,UINT32 size) noexcept {
    if(!raw || !status || size!=sizeof(*status))return E_INVALIDARG;
    *status=static_cast<Instance*>(raw)->output.status();return S_OK;
}
HRESULT WINAPI TxTimedEvent(void* raw,HANDLE* event) noexcept {
    if(!raw || !event)return E_INVALIDARG;
    *event=static_cast<Instance*>(raw)->output.event();return *event?S_OK:E_OUTOFMEMORY;
}
void WINAPI TxTimedCancel(void* raw) noexcept { if(raw)static_cast<Instance*>(raw)->output.cancel(); }
HRESULT WINAPI TxTimedStop(void* raw) noexcept {
    const auto hr=owner(raw);if(FAILED(hr))return hr;
    static_cast<Instance*>(raw)->output.stop();return S_OK;
}
HRESULT WINAPI TxTimedDestroy(void* raw) noexcept {
    const auto hr=owner(raw);if(FAILED(hr))return hr;
    delete static_cast<Instance*>(raw);return S_OK;
}
}
