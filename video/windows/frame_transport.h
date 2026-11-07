#pragma once
#include "video_contract.h"
#include "frame_mailbox.h"
#include <atomic>
#include <memory>
#include <mutex>
#include <thread>

namespace tingxian::video {
inline constexpr wchar_t PipePrefix[] = L"\\\\.\\pipe\\TingXian.Video.";
inline constexpr DWORD PipeIoTimeoutMs = 500;
// Transient MF activation configuration, supplied only by an explicit output start.
inline constexpr GUID PipeEndpointAttribute = {0xa69b3b97, 0x6946, 0x4f8b, {0xac,0xdb,0x96,0xc2,0xef,0xb8,0x93,0x51}};

struct VideoPipeEndpoint {
    UINT32 version = 1;
    wchar_t name[96]{};
    DWORD producer_pid = 0;
    DWORD producer_session = 0;
    BYTE capability[32]{};
};
enum class TransportState : UINT32 { disconnected, connected, streaming, stopped, fault };
bool valid_video_endpoint(const VideoPipeEndpoint& endpoint) noexcept;
struct TransportStatus {
    TransportState state = TransportState::disconnected;
    HRESULT last_error = S_OK;
};

MIDL_INTERFACE("24c8ea48-4ae9-4a19-b329-067c66e9919e") IFrameTransport : public IUnknown {
    virtual HRESULT STDMETHODCALLTYPE ConnectPipe(const VideoPipeEndpoint* endpoint) = 0;
    virtual HRESULT STDMETHODCALLTYPE DisconnectPipe() = 0;
    virtual HRESULT STDMETHODCALLTYPE GetTransportStatus(TransportStatus* status) = 0;
};

// A producer process owns the pipe. Only its logon SID and LocalService get
// client access. Neither the endpoint nor its random capability belongs in logs
// or persisted preferences. open/accept/begin are explicit, never startup hooks.
class FramePublisher {
    friend struct TransportTestPeer; // Native test peer only; not a runtime entry point.
    HANDLE pipe_ = INVALID_HANDLE_VALUE;
    HANDLE stop_ = nullptr;
    std::mutex writer_;
    VideoPipeEndpoint endpoint_{};
    bool connected_ = false;
    HRESULT send(UINT16 kind, UINT64 epoch, UINT64 id, UINT64 captured, const BYTE* data, UINT32 bytes);
public:
    FramePublisher();
    ~FramePublisher();
    FramePublisher(const FramePublisher&) = delete;
    FramePublisher& operator=(const FramePublisher&) = delete;
    HRESULT open(VideoPipeEndpoint* endpoint);
    HRESULT accept();
    HRESULT begin(UINT64 epoch);
    HRESULT submit(UINT64 epoch, UINT64 id, UINT64 captured, const BYTE* data, UINT32 bytes);
    HRESULT end(UINT64 epoch);
    void cancel() noexcept; // event only; caller may close/join on its owning lane
    void close();
};

class PipeReceiver {
    std::shared_ptr<FrameMailbox> mailbox_;
    HANDLE pipe_ = INVALID_HANDLE_VALUE;
    HANDLE stop_ = nullptr;
    std::thread reader_;
    std::mutex lifecycle_;
    std::atomic<TransportState> state_{TransportState::disconnected};
    std::atomic<HRESULT> error_{S_OK};
    void disconnect_locked();
    void receive();
public:
    explicit PipeReceiver(std::shared_ptr<FrameMailbox> mailbox);
    ~PipeReceiver();
    HRESULT connect(const VideoPipeEndpoint& endpoint);
    void disconnect();
    TransportStatus status() const { return {state_.load(), error_.load()}; }
};
}
