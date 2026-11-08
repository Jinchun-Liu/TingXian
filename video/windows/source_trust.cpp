#include "source_trust.h"
#include "source_pin.h"
#include <aclapi.h>
#include <bcrypt.h>
#include <sddl.h>
#include <shlobj.h>
#include <cstring>

namespace tingxian::video {
namespace {
struct Local { void* value = nullptr; ~Local() { if (value) LocalFree(value); } };
bool trusted_sid(PSID sid, PSID installer) noexcept {
    return sid && IsValidSid(sid) && (IsWellKnownSid(sid, WinLocalSystemSid)
        || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) || EqualSid(sid, installer));
}
}
bool protected_source_descriptor(PSECURITY_DESCRIPTOR descriptor, bool registry) noexcept {
    if (!descriptor || !IsValidSecurityDescriptor(descriptor)) return false;
    PSID installer = nullptr;
    if (!ConvertStringSidToSidW(L"S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464", &installer)) return false;
    Local owned{installer};
    PSID owner = nullptr; BOOL defaulted = FALSE;
    if (!GetSecurityDescriptorOwner(descriptor, &owner, &defaulted) || !trusted_sid(owner, installer)) return false;
    PACL acl = nullptr; BOOL present = FALSE;
    if (!GetSecurityDescriptorDacl(descriptor, &present, &acl, &defaulted) || !present || !acl || !IsValidAcl(acl)) return false;
    // File and registry rights reuse bit positions (e.g. execute/create-link).
    const DWORD writes = DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_WRITE | GENERIC_ALL
        | (registry ? KEY_SET_VALUE | KEY_CREATE_SUB_KEY | KEY_CREATE_LINK
            : FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES | FILE_DELETE_CHILD);
    for (DWORD i = 0; i < acl->AceCount; ++i) {
        void* value = nullptr;
        if (!GetAce(acl, i, &value)) return false;
        auto header = static_cast<ACE_HEADER*>(value);
        if (header->AceFlags & INHERIT_ONLY_ACE) continue;
        if (header->AceType == ACCESS_DENIED_ACE_TYPE) continue;
        // Unknown/conditional/object grants are not silently treated as safe.
        if (header->AceType != ACCESS_ALLOWED_ACE_TYPE) return false;
        auto allow = static_cast<ACCESS_ALLOWED_ACE*>(value);
        if ((allow->Mask & writes) && !trusted_sid(&allow->SidStart, installer)) return false;
    }
    return true;
}
HRESULT check_source_security(HANDLE object, bool registry) noexcept {
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    DWORD error = GetSecurityInfo(object, registry ? SE_REGISTRY_KEY : SE_FILE_OBJECT,
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, nullptr, nullptr, nullptr, nullptr, &descriptor);
    Local owned{descriptor};
    if (error != ERROR_SUCCESS) return HRESULT_FROM_WIN32(error);
    return protected_source_descriptor(descriptor, registry) ? S_OK : E_ACCESSDENIED;
}
HRESULT check_source_digest(HANDLE file, const char* expected) noexcept {
    if (!expected || strlen(expected) != 64) return E_INVALIDARG;
    LARGE_INTEGER size{}, zero{};
    if (!GetFileSizeEx(file, &size) || !SetFilePointerEx(file, zero, nullptr, FILE_BEGIN)) return HRESULT_FROM_WIN32(GetLastError());
    if (size.QuadPart <= 0 || size.QuadPart > 32 * 1024 * 1024) return HRESULT_FROM_WIN32(ERROR_FILE_INVALID);
    BCRYPT_ALG_HANDLE algorithm = nullptr; BCRYPT_HASH_HANDLE hash = nullptr;
    NTSTATUS status = BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0);
    if (status >= 0) status = BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0);
    HRESULT result = status >= 0 ? S_OK : E_FAIL;
    BYTE buffer[65536]{}; LONGLONG consumed = 0;
    while (SUCCEEDED(result) && consumed < size.QuadPart) {
        DWORD read = 0;
        if (!ReadFile(file, buffer, sizeof(buffer), &read, nullptr)) { result = HRESULT_FROM_WIN32(GetLastError()); break; }
        if (!read || BCryptHashData(hash, buffer, read, 0) < 0) { result = E_FAIL; break; }
        consumed += read;
    }
    BYTE digest[32]{};
    if (SUCCEEDED(result) && (consumed != size.QuadPart || BCryptFinishHash(hash, digest, sizeof(digest), 0) < 0)) result = E_FAIL;
    if (hash) BCryptDestroyHash(hash);
    if (algorithm) BCryptCloseAlgorithmProvider(algorithm, 0);
    if (FAILED(result)) return result;
    constexpr char hex[] = "0123456789abcdef";
    for (unsigned i = 0; i < 32; ++i)
        if (expected[i*2] != hex[digest[i] >> 4] || expected[i*2+1] != hex[digest[i] & 15]) return HRESULT_FROM_WIN32(ERROR_CRC);
    return S_OK;
}
void SourceLease::release() noexcept {
    for (int i = 3; i >= 0; --i) { if (handles_[i]) CloseHandle(handles_[i]); handles_[i] = nullptr; }
}
HRESULT SourceLease::acquire(const wchar_t* registered_path) noexcept {
    if (!registered_path) return E_POINTER;
    for (auto h : handles_) if (h) return HRESULT_FROM_WIN32(ERROR_BUSY);
    PWSTR program = nullptr;
    HRESULT result = SHGetKnownFolderPath(FOLDERID_ProgramFilesX64, KF_FLAG_DONT_VERIFY, nullptr, &program);
    if (FAILED(result)) return result;
    wchar_t path[32768]{};
    const bool copied = wcscpy_s(path, program) == 0;
    CoTaskMemFree(program);
    if (!copied) return HRESULT_FROM_WIN32(ERROR_BAD_PATHNAME);
    const wchar_t* suffixes[] = {L"", L"\\TingXian", L"\\VideoSource", L"\\TingXianVideoSource.dll"};
    for (unsigned i = 0; i < 4; ++i) {
        if (wcscat_s(path, suffixes[i])) { result = HRESULT_FROM_WIN32(ERROR_BAD_PATHNAME); break; }
        if (i == 3 && _wcsicmp(path, registered_path) != 0) { result = HRESULT_FROM_WIN32(ERROR_BAD_PATHNAME); break; }
        const bool directory = i < 3;
        HANDLE opened = CreateFileW(path, directory ? READ_CONTROL | FILE_READ_ATTRIBUTES : GENERIC_READ,
            directory ? FILE_SHARE_READ | FILE_SHARE_WRITE : FILE_SHARE_READ, nullptr, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | (directory ? FILE_FLAG_BACKUP_SEMANTICS : 0), nullptr);
        if (opened == INVALID_HANDLE_VALUE) { result = HRESULT_FROM_WIN32(GetLastError()); break; }
        handles_[i] = opened;
        BY_HANDLE_FILE_INFORMATION info{};
        if (!GetFileInformationByHandle(opened, &info)) { result = HRESULT_FROM_WIN32(GetLastError()); break; }
        if ((info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT)
            || ((info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) != directory
            || (!directory && info.nNumberOfLinks != 1)) { result = E_ACCESSDENIED; break; }
        result = check_source_security(opened, false);
        if (FAILED(result)) break;
        if (!directory) result = check_source_digest(opened, TingXianSourceSha256);
    }
    if (FAILED(result)) release();
    return result;
}
}
