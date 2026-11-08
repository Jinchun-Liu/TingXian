// Synthetic data and security descriptors only. Never loads a DLL or a device.
#include "source_trust.h"
#include <sddl.h>
#include <iostream>
#include <stdexcept>
using namespace tingxian::video;
namespace {
unsigned checks = 0;
void expect(bool value, const char* why) { ++checks; if (!value) throw std::runtime_error(why); }
void acl(const wchar_t* sddl, bool registry, bool accepted) {
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    expect(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &descriptor, nullptr) != FALSE, "parse synthetic ACL");
    const bool result = protected_source_descriptor(descriptor, registry);
    LocalFree(descriptor);
    expect(result == accepted, "wrong ACL decision");
}
}
int wmain(int argc, wchar_t** argv) {
    try {
        if (argc == 3 && wcscmp(argv[1], L"--installed-lease") == 0) {
            SourceLease lease;
            expect(lease.acquire(argv[2]) == S_OK, "installed source not pinned");
            HANDLE write = CreateFileW(argv[2], GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                nullptr, OPEN_EXISTING, 0, nullptr);
            expect(write == INVALID_HANDLE_VALUE && GetLastError() == ERROR_SHARING_VIOLATION, "installed source changed while leased");
            HANDLE replace = CreateFileW(argv[2], DELETE, FILE_SHARE_READ | FILE_SHARE_DELETE,
                nullptr, OPEN_EXISTING, 0, nullptr);
            expect(replace == INVALID_HANDLE_VALUE && GetLastError() == ERROR_SHARING_VIOLATION, "installed source replaced while leased");
            lease.release();
            write = CreateFileW(argv[2], GENERIC_WRITE, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr);
            expect(write != INVALID_HANDLE_VALUE, "lease retained after release");
            CloseHandle(write); // no bytes written
            std::cout << "PASS installed source lease denies write/delete then releases; no source load or device\n";
            return 0;
        }
        if (argc != 1) return 2;
        acl(L"O:BAD:(A;;FA;;;SY)(A;;FA;;;BA)(A;;FRFX;;;BU)", false, true);
        acl(L"O:BAD:(A;;KA;;;SY)(A;;KA;;;BA)(A;;KR;;;BU)", true, true);
        acl(L"O:BAD:(A;;FRFX;;;BU)(A;;FW;;;BU)", false, false);
        acl(L"O:BAD:(A;;KR;;;BU)(A;;KW;;;BU)", true, false);
        acl(L"O:BAD:(A;;GW;;;BU)", false, false);
        acl(L"O:BAD:(A;;GA;;;BU)", true, false);
        acl(L"O:BUD:(A;;FR;;;BU)", false, false); // user owner can rewrite ACL
        acl(L"O:BAD:NO_ACCESS_CONTROL", false, false);
        acl(L"O:BAD:(A;OIIO;GA;;;CO)(A;;FA;;;BA)(A;;FRFX;;;BU)", false, true);
        expect(!protected_source_descriptor(nullptr, false), "missing descriptor accepted");
        wchar_t folder[MAX_PATH]{}; expect(GetTempPathW(MAX_PATH, folder) != 0, "temp path");
        wchar_t name[MAX_PATH]{};
        swprintf_s(name, L"%stingxian-source-digest-%lu-%llu.txt", folder, GetCurrentProcessId(), GetTickCount64());
        HANDLE file = CreateFileW(name, GENERIC_READ | GENERIC_WRITE, 0, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
        expect(file != INVALID_HANDLE_VALUE, "create only new synthetic text");
        DWORD written = 0;
        expect(WriteFile(file, "abc", 3, &written, nullptr) && written == 3, "write synthetic bytes");
        constexpr char abc[] = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        expect(check_source_digest(file, abc) == S_OK, "known SHA-256");
        expect(check_source_digest(file, abc) == S_OK, "repeated hashing rewinds same handle");
        expect(check_source_digest(file, "bad") == E_INVALIDARG, "invalid pin");
        expect(check_source_digest(file, "0000000000000000000000000000000000000000000000000000000000000000") == HRESULT_FROM_WIN32(ERROR_CRC), "changed bytes accepted");
        CloseHandle(file);
        file = CreateFileW(name, GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        expect(file != INVALID_HANDLE_VALUE, "read lease");
        HANDLE mutation = CreateFileW(name, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, 0, nullptr);
        expect(mutation == INVALID_HANDLE_VALUE && GetLastError() == ERROR_SHARING_VIOLATION, "writer admitted during lease");
        HANDLE replacement = CreateFileW(name, DELETE, FILE_SHARE_READ | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING, 0, nullptr);
        expect(replacement == INVALID_HANDLE_VALUE && GetLastError() == ERROR_SHARING_VIOLATION, "replacement admitted during lease");
        expect(check_source_digest(file, abc) == S_OK, "lease digest changed");
        CloseHandle(file); // leave the tiny synthetic text in the disposable runner
        std::cout << "PASS source byte pin, read lease and ACL boundaries: " << checks << " assertions; no DLL loaded or device created\n";
        return 0;
    } catch (const std::exception& e) { std::cerr << "FAIL " << e.what() << '\n'; return 1; }
}
