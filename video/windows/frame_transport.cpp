#include "frame_transport.h"
#include <sddl.h>
#include <bcrypt.h>
#include <array>
#include <cstring>
#include <string>
#include <vector>

namespace tingxian::video {
namespace {
constexpr UINT32 Magic = 0x31565854; // TXV1, little endian Windows x64
enum : UINT16 { Hello = 1, Ack = 2, Begin = 3, Frame = 4, End = 5, ScheduledFrame = 6, Receipt = 7 };
struct Header {
    UINT32 magic = Magic;
    UINT16 version = 1;
    UINT16 kind = 0;
    UINT32 bytes = 0;
    UINT32 reserved = 0;
    UINT64 epoch = 0;
    UINT64 id = 0;
    UINT64 captured = 0;
};
static_assert(sizeof(Header) == 40, "wire header layout changed");
HRESULT last_error() { return HRESULT_FROM_WIN32(GetLastError()); }
struct Handle {
    HANDLE value = nullptr;
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
};
struct LocalAllocation {
    void* value = nullptr;
    ~LocalAllocation() { if (value) LocalFree(value); }
};
struct Revert {
    ~Revert() { if (!RevertToSelf()) RaiseFailFastException(nullptr, nullptr, 0); }
};

HRESULT transfer(HANDLE pipe, bool write, void* data, DWORD size, HANDLE stop) {
    const UINT64 deadline = GetTickCount64() + PipeIoTimeoutMs;
    auto bytes = static_cast<BYTE*>(data);
    DWORD completed = 0;
    while (completed < size) {
        if (WaitForSingleObject(stop, 0) == WAIT_OBJECT_0) return HRESULT_FROM_WIN32(ERROR_CANCELLED);
        Handle event{CreateEventW(nullptr, TRUE, FALSE, nullptr)};
        if (!event.value) return last_error();
        OVERLAPPED operation{}; operation.hEvent = event.value;
        DWORD count = 0;
        BOOL ok = write ? WriteFile(pipe, bytes + completed, size - completed, &count, &operation)
                        : ReadFile(pipe, bytes + completed, size - completed, &count, &operation);
        if (!ok && GetLastError() != ERROR_IO_PENDING) return last_error();
        if (!ok) {
            HANDLE events[] = {stop, event.value};
            const auto now = GetTickCount64();
            DWORD wait = WaitForMultipleObjects(2, events, FALSE, now >= deadline ? 0 : DWORD(deadline - now));
            if (wait != WAIT_OBJECT_0 + 1) {
                CancelIoEx(pipe, &operation);
                GetOverlappedResult(pipe, &operation, &count, TRUE); // retire stack OVERLAPPED before returning
                return HRESULT_FROM_WIN32(wait == WAIT_TIMEOUT ? ERROR_TIMEOUT : ERROR_CANCELLED);
            }
            if (!GetOverlappedResult(pipe, &operation, &count, FALSE)) return last_error();
        }
        if (count == 0) return HRESULT_FROM_WIN32(ERROR_BROKEN_PIPE);
        completed += count;
    }
    return S_OK;
}

HRESULT connect_server(HANDLE pipe, HANDLE stop) {
    Handle event{CreateEventW(nullptr, TRUE, FALSE, nullptr)};
    if (!event.value) return last_error();
    OVERLAPPED operation{}; operation.hEvent = event.value;
    if (ConnectNamedPipe(pipe, &operation)) return S_OK;
    DWORD error = GetLastError();
    if (error == ERROR_PIPE_CONNECTED) return S_OK;
    if (error != ERROR_IO_PENDING) return HRESULT_FROM_WIN32(error);
    HANDLE events[] = {stop, event.value};
    // Meeting software may activate the source much later. The producer's close()
    // cancels this wait; no user action or frame is inferred from elapsed time.
    DWORD wait = WaitForMultipleObjects(2, events, FALSE, INFINITE);
    DWORD count = 0;
    if (wait != WAIT_OBJECT_0 + 1) {
        CancelIoEx(pipe, &operation); GetOverlappedResult(pipe, &operation, &count, TRUE);
        return HRESULT_FROM_WIN32(wait == WAIT_TIMEOUT ? ERROR_TIMEOUT : ERROR_CANCELLED);
    }
    return GetOverlappedResult(pipe, &operation, &count, FALSE) ? S_OK : last_error();
}

HRESULT token_information(HANDLE token, TOKEN_INFORMATION_CLASS type, std::vector<BYTE>& bytes) {
    DWORD size = 0;
    GetTokenInformation(token, type, nullptr, 0, &size);
    if (GetLastError() != ERROR_INSUFFICIENT_BUFFER) return last_error();
    bytes.resize(size);
    return GetTokenInformation(token, type, bytes.data(), size, &size) ? S_OK : last_error();
}

HRESULT logon_sid(HANDLE token, std::wstring& sid) {
    std::vector<BYTE> bytes;
    HRESULT hr = token_information(token, TokenGroups, bytes); if (FAILED(hr)) return hr;
    auto groups = reinterpret_cast<TOKEN_GROUPS*>(bytes.data());
    for (DWORD i = 0; i < groups->GroupCount; ++i) {
        if ((groups->Groups[i].Attributes & SE_GROUP_LOGON_ID) == SE_GROUP_LOGON_ID) {
            LPWSTR text = nullptr;
            if (!ConvertSidToStringSidW(groups->Groups[i].Sid, &text)) return last_error();
            LocalAllocation allocation{text}; sid = text; return S_OK;
        }
    }
    return E_ACCESSDENIED;
}

HRESULT current_logon(std::wstring& sid) {
    Handle token;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token.value)) return last_error();
    return logon_sid(token.value, sid);
}

HRESULT authenticate_client(HANDLE pipe) {
    std::wstring owner;
    HRESULT hr = current_logon(owner); if (FAILED(hr)) return hr;
    if (!ImpersonateNamedPipeClient(pipe)) return last_error();
    Revert revert;
    Handle token;
    if (!OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &token.value)) return last_error();
    std::vector<BYTE> user;
    hr = token_information(token.value, TokenUser, user); if (FAILED(hr)) return hr;
    BYTE service[SECURITY_MAX_SID_SIZE]; DWORD service_size = sizeof(service);
    if (!CreateWellKnownSid(WinLocalServiceSid, nullptr, service, &service_size)) return last_error();
    if (EqualSid(reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid, service)) return S_OK;
    std::wstring peer;
    hr = logon_sid(token.value, peer);
    return SUCCEEDED(hr) && peer == owner ? S_OK : E_ACCESSDENIED;
}

}
bool valid_video_endpoint(const VideoPipeEndpoint& endpoint) noexcept {
    constexpr size_t prefix_size = _countof(PipePrefix) - 1;
    const size_t length = wcsnlen_s(endpoint.name, _countof(endpoint.name));
    if ((endpoint.version != 1 && endpoint.version != 2) || !endpoint.producer_pid || length != prefix_size + 32
        || wcsncmp(endpoint.name, PipePrefix, prefix_size) != 0) return false;
    for (size_t i = prefix_size; i < length; ++i) {
        wchar_t c = endpoint.name[i];
        if (!((c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'f'))) return false;
    }
    BYTE any = 0; for (BYTE byte : endpoint.capability) any |= byte;
    return any != 0;
}
namespace {
bool basic_header(const Header& header, UINT16 version) { return header.magic == Magic && header.version == version && header.reserved == 0; }
bool control_header(const Header& header, UINT16 version, UINT16 kind, UINT32 bytes = 0) {
    return basic_header(header, version) && header.kind == kind && header.bytes == bytes
        && header.epoch == 0 && header.id == 0 && header.captured == 0;
}
}

FramePublisher::FramePublisher() { stop_ = CreateEventW(nullptr, TRUE, FALSE, nullptr); }
FramePublisher::~FramePublisher() { close(); if (stop_) CloseHandle(stop_); SecureZeroMemory(&endpoint_, sizeof(endpoint_)); }
HRESULT FramePublisher::open(VideoPipeEndpoint* endpoint) {
    return open_protocol(endpoint, 1);
}
HRESULT FramePublisher::open_timed(VideoPipeEndpoint* endpoint) {
    return open_protocol(endpoint, 2);
}
HRESULT FramePublisher::open_protocol(VideoPipeEndpoint* endpoint, UINT16 version) {
    if (!endpoint) return E_POINTER;
    close();
    std::lock_guard guard(writer_);
    if (!stop_) return E_OUTOFMEMORY;
    ResetEvent(stop_); endpoint_ = {};
    endpoint_.version = version;
    std::array<BYTE, 16> random{};
    if (BCryptGenRandom(nullptr, random.data(), ULONG(random.size()), BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0
        || BCryptGenRandom(nullptr, endpoint_.capability, sizeof(endpoint_.capability), BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0) return E_FAIL;
    std::wstring name = PipePrefix;
    const wchar_t digits[] = L"0123456789abcdef";
    for (BYTE b : random) { name += digits[b >> 4]; name += digits[b & 15]; }
    wcscpy_s(endpoint_.name, name.c_str());
    endpoint_.producer_pid = GetCurrentProcessId();
    if (!ProcessIdToSessionId(endpoint_.producer_pid, &endpoint_.producer_session)) return last_error();
    std::wstring logon;
    HRESULT hr = current_logon(logon); if (FAILED(hr)) return hr;
    // LocalService gets individual read/write rights, not FILE_CREATE_PIPE_INSTANCE.
    const std::wstring sddl = L"D:P(A;;GA;;;" + logon + L")(A;;0x12019b;;;LS)";
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1, &descriptor, nullptr)) return last_error();
    LocalAllocation allocation{descriptor};
    SECURITY_ATTRIBUTES security{sizeof(security), descriptor, FALSE};
    pipe_ = CreateNamedPipeW(endpoint_.name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 128 * 1024, 4096, 0, &security);
    if (pipe_ == INVALID_HANDLE_VALUE) return last_error();
    *endpoint = endpoint_;
    return S_OK;
}
HRESULT FramePublisher::accept() {
    std::lock_guard guard(writer_);
    if (pipe_ == INVALID_HANDLE_VALUE) return E_UNEXPECTED;
    HRESULT hr = connect_server(pipe_, stop_); if (FAILED(hr)) return hr;
    Header hello{}; std::array<BYTE, 32> capability{};
    hr = transfer(pipe_, false, &hello, sizeof(hello), stop_); if (FAILED(hr)) return hr;
    if (!control_header(hello, UINT16(endpoint_.version), Hello, DWORD(capability.size()))) return E_ACCESSDENIED;
    hr = transfer(pipe_, false, capability.data(), DWORD(capability.size()), stop_); if (FAILED(hr)) return hr;
    BYTE difference = 0;
    for (size_t i = 0; i < capability.size(); ++i) difference |= capability[i] ^ endpoint_.capability[i];
    SecureZeroMemory(capability.data(), capability.size());
    if (difference) return E_ACCESSDENIED;
    hr = authenticate_client(pipe_); if (FAILED(hr)) return hr;
    Header ack{}; ack.kind = Ack; ack.version = UINT16(endpoint_.version);
    hr = transfer(pipe_, true, &ack, sizeof(ack), stop_);
    connected_ = SUCCEEDED(hr); return hr;
}
HRESULT FramePublisher::send(UINT16 kind, UINT64 epoch, UINT64 id, UINT64 captured, const BYTE* data, UINT32 bytes, bool* received) {
    if (received) *received = false;
    std::lock_guard guard(writer_);
    if (!connected_ || pipe_ == INVALID_HANDLE_VALUE) return HRESULT_FROM_WIN32(ERROR_PIPE_NOT_CONNECTED);
    if ((kind == Frame && endpoint_.version != 1) || (kind == ScheduledFrame && endpoint_.version != 2)) return E_INVALIDARG;
    Header header{}; header.kind = kind; header.epoch = epoch; header.id = id; header.captured = captured; header.bytes = bytes;
    header.version = UINT16(endpoint_.version);
    HRESULT hr = transfer(pipe_, true, &header, sizeof(header), stop_);
    if (SUCCEEDED(hr) && bytes) hr = transfer(pipe_, true, const_cast<BYTE*>(data), bytes, stop_);
    HRESULT admitted = S_OK;
    if (SUCCEEDED(hr) && endpoint_.version == 2) {
        Header receipt{};
        hr = transfer(pipe_, false, &receipt, sizeof(receipt), stop_);
        if (SUCCEEDED(hr) && (!basic_header(receipt, 2) || receipt.kind != Receipt || receipt.bytes != sizeof(HRESULT) ||
            receipt.epoch != epoch || receipt.id != id || receipt.captured)) hr = E_INVALIDARG;
        if (SUCCEEDED(hr)) hr = transfer(pipe_, false, &admitted, sizeof(admitted), stop_);
        // v2 admission is exactly S_OK or a failing HRESULT. An undefined positive
        // status is a protocol failure, not evidence that the receiver admitted it.
        if (SUCCEEDED(hr) && admitted != S_OK && SUCCEEDED(admitted)) hr = E_INVALIDARG;
        if (SUCCEEDED(hr) && received) *received = true;
    }
    if (FAILED(hr)) { connected_ = false; DisconnectNamedPipe(pipe_); }
    // A negative admission is not a transport fault. No receipt proves delivery
    // by MF, an audio endpoint or a meeting receiver.
    return FAILED(hr) ? hr : admitted;
}
HRESULT FramePublisher::begin(UINT64 epoch) { return epoch ? send(Begin, epoch, 0, 0, nullptr, 0) : E_INVALIDARG; }
HRESULT FramePublisher::submit(UINT64 epoch, UINT64 id, UINT64 captured, const BYTE* data, UINT32 bytes) {
    if (!epoch || !id || !data || bytes != FrameBytes) return E_INVALIDARG;
    return send(Frame, epoch, id, captured, data, bytes);
}
HRESULT FramePublisher::submit_timed(UINT64 epoch, UINT64 sequence, const TimedFrame& timing, const BYTE* data, UINT32 bytes, bool* receipt) {
    if (receipt) *receipt = false;
    if (!epoch || !sequence || timing.flags > TimedNeutral || timing.reserved ||
        (timing.flags == TimedNeutral ? (data || bytes) : (!data || bytes != FrameBytes))) return E_INVALIDARG;
    try {
        std::vector<BYTE> payload(sizeof(timing) + bytes);
        std::memcpy(payload.data(), &timing, sizeof(timing));
        if (bytes) std::memcpy(payload.data() + sizeof(timing), data, bytes);
        const auto result = send(ScheduledFrame, epoch, sequence, 0, payload.data(), UINT32(payload.size()), receipt);
        SecureZeroMemory(payload.data(), payload.size()); return result;
    } catch (const std::bad_alloc&) { return E_OUTOFMEMORY; }
}
HRESULT FramePublisher::end(UINT64 epoch) { return epoch ? send(End, epoch, 0, 0, nullptr, 0) : E_INVALIDARG; }
void FramePublisher::cancel() noexcept {
    if (stop_) SetEvent(stop_);
}
void FramePublisher::close() {
    cancel();
    std::lock_guard guard(writer_);
    connected_ = false;
    if (pipe_ != INVALID_HANDLE_VALUE) { DisconnectNamedPipe(pipe_); CloseHandle(pipe_); pipe_ = INVALID_HANDLE_VALUE; }
    SecureZeroMemory(endpoint_.capability, sizeof(endpoint_.capability));
}

PipeReceiver::PipeReceiver(std::shared_ptr<FrameMailbox> mailbox) : mailbox_(std::move(mailbox)) {
    stop_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
}
PipeReceiver::~PipeReceiver() { disconnect(); if (stop_) CloseHandle(stop_); }
void PipeReceiver::disconnect_locked() {
    if (stop_) SetEvent(stop_);
    if (reader_.joinable()) reader_.join();
    if (pipe_ != INVALID_HANDLE_VALUE) { CloseHandle(pipe_); pipe_ = INVALID_HANDLE_VALUE; }
    mailbox_->stop(); state_ = TransportState::disconnected;
}
void PipeReceiver::disconnect() { std::lock_guard guard(lifecycle_); disconnect_locked(); }
HRESULT PipeReceiver::connect(const VideoPipeEndpoint& endpoint) {
    std::lock_guard guard(lifecycle_);
    disconnect_locked(); error_ = S_OK;
    auto fail = [&](HRESULT hr) {
        if (pipe_ != INVALID_HANDLE_VALUE) { CloseHandle(pipe_); pipe_ = INVALID_HANDLE_VALUE; }
        error_ = hr; state_ = TransportState::fault; mailbox_->stop(); return hr;
    };
    if (!stop_) return fail(E_OUTOFMEMORY);
    if (!valid_video_endpoint(endpoint)) return fail(E_INVALIDARG);
    ResetEvent(stop_);
    pipe_ = CreateFileW(endpoint.name, FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0, nullptr, OPEN_EXISTING, FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, nullptr);
    if (pipe_ == INVALID_HANDLE_VALUE) return fail(last_error());
    ULONG pid = 0, session = 0;
    if (!GetNamedPipeServerProcessId(pipe_, &pid) || !GetNamedPipeServerSessionId(pipe_, &session)) return fail(last_error());
    if (pid != endpoint.producer_pid || session != endpoint.producer_session) return fail(E_ACCESSDENIED);
    protocol_ = UINT16(endpoint.version);
    Header hello{}; hello.kind = Hello; hello.bytes = sizeof(endpoint.capability); hello.version = protocol_;
    HRESULT hr = transfer(pipe_, true, &hello, sizeof(hello), stop_);
    if (SUCCEEDED(hr)) hr = transfer(pipe_, true, const_cast<BYTE*>(endpoint.capability), sizeof(endpoint.capability), stop_);
    Header ack{};
    if (SUCCEEDED(hr)) hr = transfer(pipe_, false, &ack, sizeof(ack), stop_);
    if (FAILED(hr)) return fail(hr);
    if (!control_header(ack, protocol_, Ack)) return fail(E_ACCESSDENIED);
    state_ = TransportState::connected;
    try { reader_ = std::thread(&PipeReceiver::receive, this); }
    catch (...) { return fail(E_OUTOFMEMORY); }
    return S_OK;
}
void PipeReceiver::receive() {
    HRESULT result = S_OK;
    try {
        std::vector<BYTE> pixels(FrameBytes);
        UINT64 epoch = 0, generation = 0; bool enabled = false;
        while (WaitForSingleObject(stop_, 0) != WAIT_OBJECT_0) {
            Header header{};
            result = transfer(pipe_, false, &header, sizeof(header), stop_);
            if (FAILED(result)) break;
            if (!basic_header(header, protocol_)) { result = E_INVALIDARG; break; }
            HRESULT admitted = S_OK;
            if (header.kind == Begin || header.kind == End) {
                if (header.bytes || header.id || header.captured || !header.epoch) { result = E_INVALIDARG; break; }
                if (header.kind == Begin) {
                    if (header.epoch <= epoch) { result = E_INVALIDARG; break; }
                    epoch = header.epoch;
                    result = protocol_ == 2 ? mailbox_->begin_timed(epoch, &generation) : mailbox_->begin(&generation);
                    if (FAILED(result)) break;
                    enabled = true; state_ = TransportState::streaming;
                } else {
                    if (!enabled || header.epoch != epoch) { result = E_INVALIDARG; break; }
                    mailbox_->stop(); enabled = false; state_ = TransportState::stopped;
                }
            } else if (header.kind == Frame && protocol_ == 1) {
                // Validate exact byte count before reading or allocating any payload.
                if (header.bytes != FrameBytes || !header.epoch || !header.id) { result = E_INVALIDARG; break; }
                result = transfer(pipe_, false, pixels.data(), FrameBytes, stop_); if (FAILED(result)) break;
                if (!enabled || header.epoch != epoch) { mailbox_->reject(); continue; }
                const auto metrics = mailbox_->metrics();
                if (!metrics.enabled || metrics.generation != generation) { result = HRESULT_FROM_WIN32(ERROR_CANCELLED); break; }
                // Invalid IDs/timestamps are dropped and counted by the mailbox.
                mailbox_->submit(generation, header.id, header.captured, pixels.data(), FrameBytes);
            } else if (header.kind == ScheduledFrame && protocol_ == 2) {
                if (!header.epoch || !header.id || header.captured ||
                    (header.bytes != sizeof(TimedFrame) && header.bytes != sizeof(TimedFrame) + FrameBytes)) {
                    result = E_INVALIDARG; break;
                }
                TimedFrame timing;
                result = transfer(pipe_, false, &timing, sizeof(timing), stop_); if (FAILED(result)) break;
                const auto bytes = header.bytes - UINT32(sizeof(timing));
                if (timing.reserved || timing.flags > TimedNeutral ||
                    (timing.flags == TimedNeutral ? bytes != 0 : bytes != FrameBytes)) { result = E_INVALIDARG; break; }
                if (bytes) { result = transfer(pipe_, false, pixels.data(), bytes, stop_); if (FAILED(result)) break; }
                UINT64 now = 0;
                if (!qpc_now(now)) { result = E_UNEXPECTED; break; }
                admitted = mailbox_->submit_timed(enabled && header.epoch == epoch ? generation : 0,
                    header.id, &timing, bytes ? pixels.data() : nullptr, bytes, now);
            } else { result = E_INVALIDARG; break; }
            if (protocol_ == 2) {
                Header receipt{}; receipt.version = 2; receipt.kind = Receipt; receipt.bytes = sizeof(admitted);
                receipt.epoch = header.epoch; receipt.id = header.id;
                result = transfer(pipe_, true, &receipt, sizeof(receipt), stop_);
                if (SUCCEEDED(result)) result = transfer(pipe_, true, &admitted, sizeof(admitted), stop_);
                if (FAILED(result)) break;
            }
        }
        SecureZeroMemory(pixels.data(), pixels.size());
    } catch (...) { result = E_OUTOFMEMORY; }
    mailbox_->stop();
    // No other thread closes this handle before joining the reader. Closing on
    // fault promptly tells the producer to stop; it cannot keep filling a pipe.
    if (pipe_ != INVALID_HANDLE_VALUE) { CloseHandle(pipe_); pipe_ = INVALID_HANDLE_VALUE; }
    error_ = result;
    state_ = WaitForSingleObject(stop_, 0) == WAIT_OBJECT_0 ? TransportState::disconnected : TransportState::fault;
}
}
