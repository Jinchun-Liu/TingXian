// Real restricted-handle inheritance, real owner and worker processes; injected
// device backend only. Does not register/open any camera, audio, GPU or network.
#include "worker_owner.h"
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

using namespace tingxian::video;
namespace {
constexpr HRESULT Cancelled = HRESULT_FROM_WIN32(ERROR_CANCELLED);
void expect(bool yes, const char* message) { if (!yes) throw std::runtime_error(message); }
struct Handle {
    HANDLE value = nullptr;
    Handle() = default;
    explicit Handle(HANDLE h) : value(h) {}
    ~Handle() { if (value) CloseHandle(value); }
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
};
HANDLE event() { HANDLE h = CreateEventW(nullptr, TRUE, FALSE, nullptr); expect(h != nullptr, "create event"); return h; }
void duplicate(HANDLE input, DWORD access, Handle& output) {
    expect(DuplicateHandle(GetCurrentProcess(), input, GetCurrentProcess(), &output.value,
        access, TRUE, 0) != FALSE, "restricted inheritable handle");
}
std::wstring number(HANDLE handle) { return std::to_wstring(reinterpret_cast<UINT_PTR>(handle)); }
HANDLE parse(const wchar_t* value) {
    wchar_t* end = nullptr; const auto n = _wcstoui64(value, &end, 10);
    expect(n != 0 && end && !*end, "invalid child handle argument"); return reinterpret_cast<HANDLE>(n);
}
struct Child {
    Handle process, diagnostics;
    ~Child() {
        if (process.value && WaitForSingleObject(process.value, 0) == WAIT_TIMEOUT) {
            // Only this test's exact child, on failure/timeout. Never target a PID
            // found by name or an existing application. Timeout still fails.
            TerminateProcess(process.value, 91); WaitForSingleObject(process.value, 5000);
        }
    }
    DWORD exit() {
        expect(WaitForSingleObject(process.value, 5000) == WAIT_OBJECT_0, "child exit timeout");
        DWORD code = 0; expect(GetExitCodeProcess(process.value, &code) != FALSE, "child exit code");
        char message[1024]{}; DWORD read = 0;
        while (ReadFile(diagnostics.value, message, sizeof(message), &read, nullptr) && read)
            std::cerr.write(message, read);
        return code;
    }
};
std::unique_ptr<Child> spawn(const std::wstring& arguments, const std::vector<HANDLE>& handles) {
    wchar_t exe[32768]{};
    const DWORD size = GetModuleFileNameW(nullptr, exe, _countof(exe));
    expect(size && size < _countof(exe), "test executable path");
    SIZE_T bytes = 0; InitializeProcThreadAttributeList(nullptr, 1, 0, &bytes);
    std::vector<BYTE> storage(bytes);
    auto attributes = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());
    expect(InitializeProcThreadAttributeList(attributes, 1, 0, &bytes) != FALSE, "initialize handle list");
    struct AttributeGuard { LPPROC_THREAD_ATTRIBUTE_LIST p; ~AttributeGuard() { DeleteProcThreadAttributeList(p); } } guard{attributes};
    auto child = std::make_unique<Child>(); Handle output;
    SECURITY_ATTRIBUTES security{sizeof(security), nullptr, TRUE};
    expect(CreatePipe(&child->diagnostics.value, &output.value, &security, 4096) != FALSE, "child diagnostics pipe");
    expect(SetHandleInformation(child->diagnostics.value, HANDLE_FLAG_INHERIT, 0) != FALSE, "private diagnostics reader");
    std::vector<HANDLE> inherited(handles); inherited.push_back(output.value);
    expect(UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        inherited.data(), inherited.size() * sizeof(HANDLE), nullptr, nullptr) != FALSE,
        "exact inherited handle list");
    STARTUPINFOEXW startup{}; startup.StartupInfo.cb = sizeof(startup); startup.lpAttributeList = attributes;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdOutput = startup.StartupInfo.hStdError = output.value;
    PROCESS_INFORMATION info{};
    std::wstring command = L"\"" + std::wstring(exe) + L"\" " + arguments;
    expect(CreateProcessW(exe, command.data(), nullptr, nullptr, TRUE,
        EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW, nullptr, nullptr, &startup.StartupInfo, &info) != FALSE,
        "create hidden test child");
    child->process.value = info.hProcess; CloseHandle(info.hThread); return child;
}
struct Lease final : OutputLease {
    std::atomic<bool> revoked{false};
    void revoke() noexcept override { revoked = true; }
};
struct Backend final : CameraBackend {
    unsigned prepared = 0, started = 0, stopped = 0, shutdowns = 0;
    bool block = false;
    HANDLE ready = nullptr, revoked = nullptr;
    HRESULT prepare(const VideoPipeEndpoint&) noexcept override { ++prepared; return S_OK; }
    HRESULT start() noexcept override {
        ++started;
        if (block) {
            SetEvent(ready);
            if (WaitForSingleObject(revoked, 5000) != WAIT_OBJECT_0) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        }
        return S_OK; // Deliberately late OS success must not restore output.
    }
    HRESULT stop() noexcept override { ++stopped; return S_OK; }
    HRESULT shutdown() noexcept override { ++shutdowns; return S_OK; }
};
VideoPipeEndpoint endpoint() {
    VideoPipeEndpoint ep;
    wcscpy_s(ep.name, L"\\\\.\\pipe\\TingXian.Video.0123456789abcdef0123456789abcdef");
    ep.producer_pid = GetCurrentProcessId(); ep.capability[0] = 1;
    expect(ProcessIdToSessionId(ep.producer_pid, &ep.producer_session) != FALSE, "session id"); return ep;
}
int worker(HANDLE owner, HANDLE stop, HANDLE ready, const std::wstring& mode) {
    Lease lease; Backend backend; CameraSession session(backend, lease);
    WorkerOwnerGuard guard(session);
    const HRESULT armed = guard.arm(owner, stop);
    // Guard owns restricted, non-inheritable duplicates. Dropping the startup
    // handles must not affect its ability to observe owner/stop.
    CloseHandle(owner); CloseHandle(stop);
    if (mode == L"prestop" || mode == L"predead") {
        expect(armed == Cancelled && lease.revoked && backend.prepared == 0, "pre-revoked owner admitted");
        expect(guard.reason() == (mode == L"prestop" ? OwnerStopReason::explicit_stop : OwnerStopReason::owner_exited), "pre-revocation cause");
        return 0;
    }
    expect(SUCCEEDED(armed), "arm owner");
    expect(guard.arm(nullptr, nullptr) == HRESULT_FROM_WIN32(ERROR_BUSY), "guard rearmed");
    backend.ready = ready; backend.revoked = guard.revoked_event(); backend.block = mode == L"pending";
    if (mode == L"late") {
        SetEvent(ready);
        expect(WaitForSingleObject(guard.revoked_event(), 5000) == WAIT_OBJECT_0, "late stop timeout");
        expect(guard.start(endpoint()) == Cancelled && backend.prepared == 0 && lease.revoked, "queued old start was accepted");
    } else {
        const auto started = guard.start(endpoint());
        if (mode == L"pending") {
            expect(started == Cancelled && lease.revoked, "late successful Start revived output");
            expect(backend.stopped == 1 && backend.shutdowns == 1, "cancelled start was not cleaned");
        } else {
            expect(SUCCEEDED(started) && !lease.revoked, "start without revocation");
            expect(guard.start(endpoint()) == HRESULT_FROM_WIN32(ERROR_BUSY), "duplicate start accepted");
            SetEvent(ready);
            expect(WaitForSingleObject(guard.revoked_event(), 5000) == WAIT_OBJECT_0 && lease.revoked, "owner loss did not revoke");
        }
    }
    expect(guard.reason() == (mode == L"death" ? OwnerStopReason::owner_exited : OwnerStopReason::explicit_stop), "wrong revocation cause");
    guard.close(); expect(SUCCEEDED(session.stop()), "explicit MTA cleanup");
    expect(backend.stopped == (mode == L"late" ? 0u : 1u) && backend.shutdowns == backend.stopped,
        "cleanup count/order");
    expect(guard.start(endpoint()) == E_UNEXPECTED, "closed guard restarted");
    return 0;
}
void run(const wchar_t* mode) {
    Handle ownerExit(event()), ownerWait;
    duplicate(ownerExit.value, SYNCHRONIZE, ownerWait);
    auto owner = spawn(L"--test-owner " + number(ownerWait.value), {ownerWait.value});
    Handle stop(event()), ready(event()), ownerInherit, stopInherit, readyInherit;
    duplicate(owner->process.value, SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, ownerInherit);
    duplicate(stop.value, SYNCHRONIZE, stopInherit);
    duplicate(ready.value, EVENT_MODIFY_STATE, readyInherit);
    const std::wstring selected(mode);
    if (selected == L"prestop") SetEvent(stop.value);
    if (selected == L"predead") { SetEvent(ownerExit.value); expect(owner->exit() == 0, "predead owner exit"); }
    auto child = spawn(L"--test-worker " + number(ownerInherit.value) + L" " + number(stopInherit.value)
        + L" " + number(readyInherit.value) + L" " + selected,
        {ownerInherit.value, stopInherit.value, readyInherit.value});
    if (selected != L"prestop" && selected != L"predead") {
        HANDLE waiting[] = {ready.value, child->process.value};
        expect(WaitForMultipleObjects(2, waiting, FALSE, 5000) == WAIT_OBJECT_0, "worker ready timeout or early failure");
        if (selected == L"death") { SetEvent(ownerExit.value); expect(owner->exit() == 0, "watched owner exit"); }
        else SetEvent(stop.value);
    }
    expect(child->exit() == 0, "worker contract failure");
    SetEvent(ownerExit.value); expect(owner->exit() == 0, "owner cleanup");
    std::wcout << L"PASS restricted-process owner lifecycle: " << selected << L'\n';
}
}
int wmain(int argc, wchar_t** argv) {
    try {
        if (argc == 3 && std::wstring(argv[1]) == L"--test-owner")
            return WaitForSingleObject(parse(argv[2]), 30000) == WAIT_OBJECT_0 ? 0 : 3;
        if (argc == 6 && std::wstring(argv[1]) == L"--test-worker")
            return worker(parse(argv[2]), parse(argv[3]), parse(argv[4]), argv[5]);
        expect(argc == 1, "unknown test arguments");
        for (const auto mode : {L"stop", L"death", L"prestop", L"predead", L"late", L"pending"}) run(mode);
        {
            Lease lease; Backend backend; CameraSession session(backend, lease); WorkerOwnerGuard guard(session);
            expect(FAILED(guard.arm(nullptr, nullptr)) && lease.revoked && backend.prepared == 0, "invalid handles did not fail closed");
            expect(FAILED(guard.start(endpoint())), "unarmed guard started");
        }
        std::cout << "PASS invalid owner fails closed without OS start\n";
        {
            Lease lease; Backend backend; CameraSession session(backend, lease);
            const auto ticket = session.operation_ticket(); session.request_stop();
            expect(session.start_if_current(endpoint(), ticket) == Cancelled && backend.prepared == 0,
                "stop before dispatched Start lost cancellation");
        }
        std::cout << "PASS cancellation ticket rejects already queued starts\nPASS groups=8\n"; return 0;
    } catch (const std::exception& error) {
        std::cerr << "FAIL " << error.what() << '\n'; return 1;
    }
}
