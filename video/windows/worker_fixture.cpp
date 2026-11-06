// Test-only process fixture, never installed. Writes only markers beside its
// copied EXE in an exclusively created synthetic test directory.
#include <windows.h>
#include <string>
#include <iostream>
int wmain(int argc, wchar_t** argv) {
    if (argc != 4 || std::wstring(argv[1]) != L"--inspect-v1") return 2;
    wchar_t path[32768]{};
    const auto count = GetModuleFileNameW(nullptr, path, _countof(path));
    if (!count || count >= _countof(path)) return 3;
    std::wstring full(path); const auto slash = full.find_last_of(L"\\/");
    const auto folder = full.substr(0, slash + 1), mode = full.substr(slash + 1);
    HANDLE marker = CreateFileW((folder + L"launched").c_str(), GENERIC_WRITE, FILE_SHARE_READ,
        nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (marker == INVALID_HANDLE_VALUE) return 4;
    CloseHandle(marker);
    if (GetEnvironmentVariableW(L"TINGXIAN_TEST_SECRET", nullptr, 0) != 0) return 66;
    HANDLE stop = reinterpret_cast<HANDLE>(_wcstoui64(argv[3], nullptr, 10));
    if (mode == L"hang.exe") Sleep(INFINITE);
    if (mode == L"wait.exe") {
        if (WaitForSingleObject(stop, 5000) != WAIT_OBJECT_0) return 7;
    }
    if (mode == L"crash.exe") return 42;
    if (mode == L"oversize.exe") { std::cout << std::string(8192, 'x') << std::flush; return 0; }
    if (mode == L"invalid.exe") { std::cout << "invalid report\n"; return 0; }
    if (mode == L"descendant.exe") {
        STARTUPINFOW si{}; si.cb = sizeof(si); PROCESS_INFORMATION pi{};
        // A job limit must reject this own child before it runs. If it doesn't,
        // clean up that exact child and fail; never target another application.
        std::wstring cmd = L"\"" + full + L"\"";
        if (CreateProcessW(full.c_str(), cmd.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW | CREATE_SUSPENDED,
            nullptr, nullptr, &si, &pi)) {
            TerminateProcess(pi.hProcess, 91); WaitForSingleObject(pi.hProcess, 1000);
            CloseHandle(pi.hThread); CloseHandle(pi.hProcess); return 65;
        }
    }
    std::cout << "{\"protocol\":1,\"api_exports_present\":true,\"machine_registration_present\":false,"
        "\"user_registration_conflict\":false,\"api_error\":0,\"registration_error\":0,"
        "\"runtime\":\"not_pinned\",\"media_started\":false}" << std::endl;
    return 0;
}
