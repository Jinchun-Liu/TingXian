// Real child processes and local pipes; synthetic pixels only. No SDK/devices.
#include "stream_protocol.h"
#include <algorithm>
#include <filesystem>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

using namespace tingxian;
namespace fs = std::filesystem;
namespace {
void require(bool ok, const char* message) { if (!ok) throw std::runtime_error(message); }
struct Handle {
    HANDLE value = nullptr;
    Handle() = default;
    explicit Handle(HANDLE h) : value(h) {}
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    ~Handle() { reset(); }
    void reset(HANDLE h = nullptr) { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); value = h; }
};
void write(HANDLE pipe, const void* data, DWORD bytes) {
    DWORD done = 0; require(WriteFile(pipe, data, bytes, &done, nullptr) && done == bytes, "pipe write failed");
}
bool read(HANDLE pipe, void* data, DWORD bytes, DWORD timeout = 3000) {
    auto target = static_cast<BYTE*>(data); DWORD done = 0;
    const auto deadline = GetTickCount64() + timeout;
    while (done < bytes && GetTickCount64() < deadline) {
        DWORD available = 0, count = 0;
        if (!PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr)) return false;
        if (!available) { Sleep(1); continue; }
        if (!ReadFile(pipe, target + done, (std::min)(available, bytes - done), &count, nullptr) || !count) return false;
        done += count;
    }
    return done == bytes;
}
template<class F> void eventually(F&& predicate, const char* message) {
    const auto deadline = GetTickCount64() + 3000;
    while (!predicate()) { require(GetTickCount64() < deadline, message); Sleep(2); }
}
std::wstring number(HANDLE h) { return std::to_wstring(reinterpret_cast<uintptr_t>(h)); }
std::wstring environment(const wchar_t* key) {
    wchar_t buffer[32768]{}; const auto count = GetEnvironmentVariableW(key, buffer, _countof(buffer));
    require(count > 0 && count < _countof(buffer), "required system directory missing"); return buffer;
}
struct Child {
    Handle process, job, stop, input, report;
    fs::path directory;
    uint64_t generation;
    Child(const fs::path& source, const fs::path& root, const wchar_t* mode, uint64_t gen, HANDLE owner_process = GetCurrentProcess()) : generation(gen) {
        directory = root / (std::to_wstring(gen) + L"-" + mode); require(fs::create_directory(directory), "fixture directory exists");
        const auto exe = directory / mode; require(fs::copy_file(source, exe), "fixture copy failed");
        SECURITY_ATTRIBUTES security{sizeof(security), nullptr, TRUE}; Handle child_input, child_report, child_owner, child_stop;
        require(CreatePipe(&child_input.value, &input.value, &security, 128 * 1024) != 0, "input pipe failed");
        require(CreatePipe(&report.value, &child_report.value, &security, 4096) != 0, "report pipe failed");
        require(SetHandleInformation(input.value, HANDLE_FLAG_INHERIT, 0) && SetHandleInformation(report.value, HANDLE_FLAG_INHERIT, 0), "parent pipe inheritance failed");
        stop.value = CreateEventW(nullptr, TRUE, FALSE, nullptr); require(stop.value != nullptr, "stop event failed");
        require(DuplicateHandle(GetCurrentProcess(), owner_process, GetCurrentProcess(), &child_owner.value, SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, TRUE, 0) != 0, "owner duplicate failed");
        require(DuplicateHandle(GetCurrentProcess(), stop.value, GetCurrentProcess(), &child_stop.value, SYNCHRONIZE, TRUE, 0) != 0, "stop duplicate failed");
        HANDLE inherit[] = {child_input.value, child_report.value, child_owner.value, child_stop.value};
        SIZE_T size = 0; InitializeProcThreadAttributeList(nullptr, 1, 0, &size);
        std::vector<BYTE> storage(size); auto attributes = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());
        require(InitializeProcThreadAttributeList(attributes, 1, 0, &size) != 0, "attributes failed");
        struct Attributes { LPPROC_THREAD_ATTRIBUTE_LIST p; ~Attributes() { DeleteProcThreadAttributeList(p); } } cleanup{attributes};
        require(UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherit, sizeof(inherit), nullptr, nullptr) != 0, "handle whitelist failed");
        job.value = CreateJobObjectW(nullptr, nullptr); require(job.value != nullptr, "job failed");
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        require(SetInformationJobObject(job.value, JobObjectExtendedLimitInformation, &limits, sizeof(limits)) != 0, "job limits failed");
        std::wstring command = L"\"" + exe.wstring() + L"\" --eye-stream-v1 " + number(child_owner.value) + L" " + number(child_stop.value)
            + L" " + number(child_report.value) + L" " + number(child_input.value) + L" 1280 720 " + std::to_wstring(generation);
        std::wstring env = L"SystemRoot=" + environment(L"SystemRoot"); env.push_back(0);
        env += L"TEMP=" + directory.wstring(); env.push_back(0); env += L"TMP=" + directory.wstring(); env.push_back(0);
        env += L"WINDIR=" + environment(L"WINDIR"); env.push_back(0); env.push_back(0);
        STARTUPINFOEXW startup{}; startup.StartupInfo.cb = sizeof(startup); startup.lpAttributeList = attributes;
        PROCESS_INFORMATION info{};
        require(CreateProcessW(exe.c_str(), command.data(), nullptr, nullptr, TRUE,
            CREATE_NO_WINDOW | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            env.data(), directory.c_str(), &startup.StartupInfo, &info) != 0, "child launch failed");
        process.value = info.hProcess; Handle thread(info.hThread);
        if (!AssignProcessToJobObject(job.value, process.value) || ResumeThread(thread.value) == DWORD(-1)) {
            TerminateProcess(process.value, 99); WaitForSingleObject(process.value, 3000); throw std::runtime_error("job assignment/resume failed");
        }
    }
    ~Child() { if (job.value) TerminateJobObject(job.value, 98); if (process.value) WaitForSingleObject(process.value, 3000); }
    eye::stream::Ready ready() {
        eye::stream::Ready ready{};
        require(read(report.value, &ready, eye::stream::ReadyBytes), "ready missing");
        require(ready.header.magic == eye::stream::ReadyMagic && ready.header.version == 1 && ready.header.kind == 1
            && ready.header.bytes == sizeof(video::VideoPipeEndpoint) && ready.header.reserved == 0
            && ready.header.generation == generation && video::valid_video_endpoint(ready.endpoint)
            && ready.endpoint.producer_pid == GetProcessId(process.value), "ready identity invalid");
        return ready;
    }
    void frame(uint64_t id, uint64_t captured) {
        eye::stream::InputHeader h{}; h.generation = generation; h.id = id; h.captured_ms = captured;
        std::vector<BYTE> pixels(eye::stream::BgrBytes, 31); write(input.value, &h, sizeof(h)); write(input.value, pixels.data(), DWORD(pixels.size()));
    }
    void finish() { eye::stream::InputHeader h{}; h.kind = eye::stream::Finish; h.bytes = 0; h.generation = generation; write(input.value, &h, sizeof(h)); }
    void cancel() { require(SetEvent(stop.value) != 0, "cancel failed"); }
    std::string result() {
        require(WaitForSingleObject(process.value, 3000) == WAIT_OBJECT_0, "child did not stop");
        DWORD code = 99; require(GetExitCodeProcess(process.value, &code) && code == 0, "child failed");
        std::string text; char buffer[4096]; DWORD count = 0;
        while (ReadFile(report.value, buffer, sizeof(buffer), &count, nullptr) && count) { text.append(buffer, count); require(text.size() <= 4096, "report overflow"); }
        require(text.find("\"protocol\":3") != std::string::npos && text.find("\"generation\":" + std::to_string(generation)) != std::string::npos, "report identity failed");
        require(text.find("TingXian.Video") == std::string::npos && text.find("capability") == std::string::npos, "report leaked endpoint");
        require(text.find("\"camera_started\":false") != std::string::npos && text.find("\"audio_started\":false") != std::string::npos
            && text.find("\"virtual_device_started\":false") != std::string::npos, "unexpected device state");
        return text;
    }
};
bool has(const std::string& text, const char* pair) { return text.find(pair) != std::string::npos; }
// Independent consumer decodes the actual wire, including capture time/id.
struct Client {
    Handle pipe;
    explicit Client(const video::VideoPipeEndpoint& endpoint) {
        pipe.value = CreateFileW(endpoint.name, GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr);
        require(pipe.value != INVALID_HANDLE_VALUE, "output connect failed");
        ULONG pid = 0; require(GetNamedPipeServerProcessId(pipe.value, &pid) && pid == endpoint.producer_pid, "output server identity failed");
        eye::stream::InputHeader hello{}; hello.magic = 0x31565854; hello.kind = 1; hello.bytes = 32;
        write(pipe.value, &hello, sizeof(hello)); write(pipe.value, endpoint.capability, 32);
        eye::stream::InputHeader ack{}, begin{};
        require(read(pipe.value, &ack, sizeof(ack)) && ack.magic == 0x31565854 && ack.kind == 2 && ack.bytes == 0, "output handshake failed");
        require(read(pipe.value, &begin, sizeof(begin)) && begin.kind == 3 && begin.generation != 0, "output begin failed");
    }
    void corrected(uint64_t id, uint64_t captured) {
        eye::stream::InputHeader header{}; std::vector<BYTE> pixels(video::FrameBytes);
        require(read(pipe.value, &header, sizeof(header)) && header.magic == 0x31565854 && header.kind == 4
            && header.id == id && header.captured_ms == captured && header.bytes == pixels.size(), "frame metadata changed");
        require(read(pipe.value, pixels.data(), DWORD(pixels.size())), "frame payload missing");
        require(std::all_of(pixels.begin(), pixels.begin() + video::Width * video::Height, [](BYTE b) { return b == 188; })
            && std::all_of(pixels.begin() + video::Width * video::Height, pixels.end(), [](BYTE b) { return b == 128; }), "raw or incorrect pixels reached output");
    }
    bool disconnected() { BYTE value = 0; return !read(pipe.value, &value, 1, 1000); }
};
struct DummyOwner {
    Handle process;
    explicit DummyOwner(const fs::path& self) {
        std::wstring command = L"\"" + self.wstring() + L"\" --synthetic-owner";
        STARTUPINFOW startup{}; startup.cb = sizeof(startup); PROCESS_INFORMATION info{};
        require(CreateProcessW(self.c_str(), command.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW, nullptr, nullptr, &startup, &info) != 0, "synthetic owner launch failed");
        process.value = info.hProcess; CloseHandle(info.hThread);
    }
    void stop() { require(TerminateProcess(process.value, 0) && WaitForSingleObject(process.value, 3000) == WAIT_OBJECT_0, "synthetic owner termination failed"); }
    ~DummyOwner() { TerminateProcess(process.value, 0); WaitForSingleObject(process.value, 3000); }
};
}
int wmain(int argc, wchar_t** argv) {
    if (argc == 2 && std::wstring(argv[1]) == L"--synthetic-owner") { Sleep(10000); return 0; }
    try {
        require(argc == 2, "supply the synthetic fixture executable");
        const auto source = fs::absolute(argv[1]);
        const auto root = fs::current_path() / (L"stream-fixtures-" + std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(GetTickCount64()));
        require(fs::create_directory(root), "fixture root exists");
        for (uint64_t generation = 1; generation <= 3; ++generation) {
            Child child(source, root, L"eye-stream.exe", generation); auto ready = child.ready(); Client client(ready.endpoint);
            SecureZeroMemory(&ready, sizeof(ready));
            const auto captured = GetTickCount64(); child.frame(19, captured); client.corrected(19, captured); child.finish();
            const auto report = child.result(); require(has(report, "\"outcome\":\"completed\"") && has(report, "\"submitted\":1") && has(report, "\"corrected\":1"), "completion counters failed");
            require(client.disconnected(), "finished output stayed connected");
        }
        std::cout << "PASS stream: three real process restarts, corrected NV12 and original frame identity/time\n";
        {
            Child child(source, root, L"eye-stream.exe", 4); auto ready = child.ready(); Client client(ready.endpoint);
            child.frame(1, GetTickCount64() - 1000); auto captured = GetTickCount64(); child.frame(2, captured); client.corrected(2, captured);
            child.frame(2, GetTickCount64()); child.finish(); auto report = child.result();
            require(has(report, "\"received\":3") && has(report, "\"rejected\":2") && has(report, "\"submitted\":1"), "stale/duplicate input was accepted");
        }
        std::cout << "PASS stream: stale and duplicate inputs rejected without refreshing timestamps\n";
        for (const wchar_t* mode : {L"eye-wait.exe", L"eye-slow.exe"}) {
            Child child(source, root, mode, 5);
            if (std::wstring(mode) == L"eye-slow.exe") { auto ready = child.ready(); child.frame(1, GetTickCount64()); }
            const auto marker = child.directory / (std::wstring(mode) == L"eye-wait.exe" ? L"preparing" : L"processing");
            eventually([&] { return fs::exists(marker); }, "provider did not reach cancel barrier");
            child.cancel(); require(has(child.result(), "\"outcome\":\"cancelled\""), "cancel lost to prepare/inference");
        }
        std::cout << "PASS stream: cancellation during preparation and processing\n";
        {
            Child child(source, root, L"eye-fail.exe", 6); auto ready = child.ready(); Client client(ready.endpoint);
            child.frame(1, GetTickCount64()); auto report = child.result();
            require(has(report, "\"outcome\":\"fault\"") && has(report, "\"corrected\":0") && has(report, "\"submitted\":0") && client.disconnected(), "provider failure leaked raw pixels");
        }
        std::cout << "PASS stream: provider failure discards raw fallback and revokes endpoint\n";
        for (int variant = 0; variant != 4; ++variant) {
            Child child(source, root, L"eye-stream.exe", 10 + variant); auto ready = child.ready();
            eye::stream::InputHeader header{}; header.generation = child.generation; header.id = 1; header.captured_ms = GetTickCount64();
            if (variant == 0) header.generation++; if (variant == 1) header.bytes++; if (variant == 2) header.reserved = 1;
            write(child.input.value, &header, sizeof(header));
            if (variant == 3) { BYTE bytes[9]{}; write(child.input.value, bytes, sizeof(bytes)); }
            auto report = child.result(); require(has(report, "\"error\":9") && has(report, "\"submitted\":0"), "malformed/truncated frame was accepted");
        }
        std::cout << "PASS stream: wrong generation/length/reserved and partial payload fail closed\n";
        {
            Child child(source, root, L"eye-stream.exe", 20); auto ready = child.ready(); Client client(ready.endpoint);
            auto captured = GetTickCount64(); child.frame(1, captured); client.corrected(1, captured); client.pipe.reset();
            // A pipe disconnection is observed on the next output operation.
            child.frame(2, GetTickCount64()); auto report = child.result();
            require(has(report, "\"outcome\":\"fault\"") && has(report, "\"error\":10"), "async output fault did not stop provider");
        }
        std::cout << "PASS stream: output failure ends provider instead of continuing unseen\n";
        {
            DummyOwner owner(fs::absolute(argv[0])); Child child(source, root, L"eye-stream.exe", 21, owner.process.value);
            auto ready = child.ready(); Client client(ready.endpoint); owner.stop();
            require(has(child.result(), "\"outcome\":\"cancelled\"") && client.disconnected(), "owner death did not revoke output");
        }
        std::cout << "PASS stream: owner exit cancels idle input and closes output\n";
        {
            Child child(source, root, L"eye-cleanup.exe", 22); auto ready = child.ready(); child.finish();
            auto report = child.result(); require(has(report, "\"cleanup_uncertain\":true") && has(report, "\"error\":8") && has(report, "\"outcome\":\"fault\""), "cleanup failure hidden");
        }
        std::cout << "PASS stream: provider cleanup failure remains a fault\n";
        return 0;
    } catch (const std::exception& e) { std::cerr << "FAIL stream: " << e.what() << "\n"; return 1; }
}
