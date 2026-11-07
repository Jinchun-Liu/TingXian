#pragma once
#include "timed_output.h"
// Separate ABI v2 exports; the existing TxOutput* ABI remains v1. Host-lane calls
// are owner-thread checked. Cancel/Status/Event permit concurrent observation;
// Destroy follows Stop and retirement of all external calls/borrowed handles.
extern "C" {
UINT32 WINAPI TxTimedAbi() noexcept;
void* WINAPI TxTimedCreate() noexcept;
HRESULT WINAPI TxTimedOpen(void*, UINT64, BYTE*, UINT32) noexcept;
HRESULT WINAPI TxTimedReserve(void*, UINT64, tingxian::video::TimedTicket*, UINT32) noexcept;
HRESULT WINAPI TxTimedStage(void*, const tingxian::video::TimedTicket*, UINT32,
    const tingxian::video::TimedFrame*, UINT32, const BYTE*, UINT32) noexcept;
HRESULT WINAPI TxTimedTake(void*, const tingxian::video::TimedTicket*, UINT32,
    tingxian::video::TimedReceipt*, UINT32) noexcept;
HRESULT WINAPI TxTimedStatus(void*, tingxian::video::TimedOutputStatus*, UINT32) noexcept;
HRESULT WINAPI TxTimedEvent(void*, HANDLE*) noexcept;
void WINAPI TxTimedCancel(void*) noexcept;
HRESULT WINAPI TxTimedStop(void*) noexcept;
HRESULT WINAPI TxTimedDestroy(void*) noexcept;
}
