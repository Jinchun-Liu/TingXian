#include "camera_session.h"
#include <mfapi.h>
#include <mferror.h>
#include <mfvirtualcamera.h>
#include <wrl/client.h>
#include <new>
#include <sddl.h>

namespace tingxian::video {
namespace {
using Microsoft::WRL::ComPtr;
constexpr wchar_t SourceId[] = L"{44D20DA7-55EC-4B4A-A767-99E97D6E5A90}";
constexpr wchar_t SourceKey[] = L"SOFTWARE\\Classes\\CLSID\\{44D20DA7-55EC-4B4A-A767-99E97D6E5A90}";
constexpr wchar_t ServerKey[] = L"SOFTWARE\\Classes\\CLSID\\{44D20DA7-55EC-4B4A-A767-99E97D6E5A90}\\InprocServer32";
using CreateCamera = decltype(&MFCreateVirtualCamera);
using SupportedCamera = decltype(&MFIsVirtualCameraTypeSupported);
struct CameraApi {
    HMODULE module = nullptr;
    CreateCamera create = nullptr;
    SupportedCamera supported = nullptr;
    ~CameraApi() { if (module) FreeLibrary(module); }
    HRESULT load() noexcept {
        if (create && supported) return S_OK;
        if (!module) module = LoadLibraryExW(L"mfsensorgroup.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
        if (!module) return HRESULT_FROM_WIN32(GetLastError());
        create = reinterpret_cast<CreateCamera>(GetProcAddress(module, "MFCreateVirtualCamera"));
        supported = reinterpret_cast<SupportedCamera>(GetProcAddress(module, "MFIsVirtualCameraTypeSupported"));
        return create && supported ? S_OK : HRESULT_FROM_WIN32(ERROR_PROC_NOT_FOUND);
    }
};
struct Key { HKEY key = nullptr; ~Key() { if (key) RegCloseKey(key); } };
struct Handle { HANDLE value = nullptr; ~Handle() { if (value) CloseHandle(value); } };
struct LocalMemory { void* value = nullptr; ~LocalMemory() { if (value) LocalFree(value); } };
HRESULT registration(CameraProbe& probe) noexcept {
    // Explicit 64-bit hives: never mistake HKCU/merged HKCR for the service install.
    Key user;
    LONG result = RegOpenKeyExW(HKEY_CURRENT_USER, SourceKey, 0, KEY_READ | KEY_WOW64_64KEY, &user.key);
    if (result == ERROR_SUCCESS) probe.user_registration_conflict = true;
    else if (result != ERROR_FILE_NOT_FOUND && result != ERROR_PATH_NOT_FOUND) return HRESULT_FROM_WIN32(result);
    Key machine;
    result = RegOpenKeyExW(HKEY_LOCAL_MACHINE, ServerKey, 0, KEY_QUERY_VALUE | KEY_WOW64_64KEY, &machine.key);
    if (result == ERROR_FILE_NOT_FOUND || result == ERROR_PATH_NOT_FOUND) return REGDB_E_CLASSNOTREG;
    if (result != ERROR_SUCCESS) return HRESULT_FROM_WIN32(result);
    wchar_t path[32768]{}; DWORD bytes = sizeof(path);
    result = RegGetValueW(machine.key, nullptr, nullptr, RRF_RT_REG_SZ | RRF_ZEROONFAILURE, nullptr, path, &bytes);
    if (result != ERROR_SUCCESS) return HRESULT_FROM_WIN32(result);
    // Local absolute path, not a DLL search name, expansion or UNC. Never log it.
    if (wcsnlen_s(path, _countof(path)) < 4 || !((path[0] >= L'A' && path[0] <= L'Z') || (path[0] >= L'a' && path[0] <= L'z'))
        || path[1] != L':' || path[2] != L'\\') return HRESULT_FROM_WIN32(ERROR_BAD_PATHNAME);
    wchar_t threading[32]{}; bytes = sizeof(threading);
    result = RegGetValueW(machine.key, nullptr, L"ThreadingModel", RRF_RT_REG_SZ | RRF_ZEROONFAILURE,
        nullptr, threading, &bytes);
    if (result != ERROR_SUCCESS) return HRESULT_FROM_WIN32(result);
    if (wcscmp(threading, L"Both") != 0) return HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
    DWORD attributes = GetFileAttributesW(path);
    if (attributes == INVALID_FILE_ATTRIBUTES) return HRESULT_FROM_WIN32(GetLastError());
    if (attributes & FILE_ATTRIBUTE_DIRECTORY) return HRESULT_FROM_WIN32(ERROR_DIRECTORY);
    probe.machine_registration_present = true;
    return probe.user_registration_conflict ? E_ACCESSDENIED : S_OK;
}

class WindowsCameraBackend final : public CameraBackend {
    CameraApi api_;
    ComPtr<IMFVirtualCamera> camera_;
    bool mf_started_ = false;
    bool start_attempted_ = false;
    bool started_ = false;
    Handle reservation_;
public:
    ~WindowsCameraBackend() override { shutdown(); }
    HRESULT prepare(const VideoPipeEndpoint& endpoint) noexcept override {
        if (camera_ || mf_started_) return HRESULT_FROM_WIN32(ERROR_BUSY);
        if (!valid_video_endpoint(endpoint)) return E_INVALIDARG;
        // Caller owns the dedicated MTA. Never initialize/uninitialize COM on
        // arbitrary UI threads or create a privacy-consent deadlock there.
        APTTYPE type; APTTYPEQUALIFIER qualifier;
        HRESULT hr = CoGetApartmentType(&type, &qualifier);
        if (FAILED(hr)) return hr;
        if (type != APTTYPE_MTA) return RPC_E_WRONG_THREAD;
        CameraProbe probe;
        hr = registration(probe); if (FAILED(hr)) return hr;
        hr = reserve_video_output(&reservation_.value); if (FAILED(hr)) return hr;
        hr = api_.load(); if (FAILED(hr)) return hr;
        hr = MFStartup(MF_VERSION, MFSTARTUP_FULL); if (FAILED(hr)) return hr;
        mf_started_ = true;
        BOOL supported = FALSE;
        hr = api_.supported(MFVirtualCameraType_SoftwareCameraSource, &supported);
        if (FAILED(hr)) return hr;
        if (!supported) return HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED);
        hr = api_.create(MFVirtualCameraType_SoftwareCameraSource, MFVirtualCameraLifetime_Session,
            MFVirtualCameraAccess_CurrentUser, L"听弦视频", SourceId, nullptr, 0, &camera_);
        if (FAILED(hr)) return hr;
        // No physical-camera wrapping. Configuration is scoped to explicit output.
        return camera_->SetBlob(PipeEndpointAttribute, reinterpret_cast<const UINT8*>(&endpoint), sizeof(endpoint));
    }
    HRESULT start() noexcept override {
        if (!camera_) return E_UNEXPECTED;
        start_attempted_ = true;
        const HRESULT hr = camera_->Start(nullptr);
        started_ = SUCCEEDED(hr);
        return hr;
    }
    HRESULT stop() noexcept override {
        const HRESULT hr = camera_ && start_attempted_ ? camera_->Stop() : S_OK;
        if (SUCCEEDED(hr)) started_ = false;
        return hr;
    }
    HRESULT preview_source(IMFMediaSource** source) noexcept override {
        if (!source) return E_POINTER;
        *source = nullptr;
        return camera_ && started_ ? camera_->GetMediaSource(source) : E_UNEXPECTED;
    }
    HRESULT symbolic_link(wchar_t** link, UINT32* length) noexcept override {
        if (!link || !length) return E_POINTER;
        *link = nullptr; *length = 0;
        return camera_ && started_ ? camera_->GetAllocatedString(
            MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, link, length) : E_UNEXPECTED;
    }
    HRESULT shutdown() noexcept override {
        HRESULT hr = S_OK;
        if (camera_) {
            const HRESULT cleared = camera_->DeleteItem(PipeEndpointAttribute);
            hr = camera_->Shutdown(); // still attempt after an attribute-clear failure
            if (FAILED(hr)) return hr; // retain this object for an explicit retry
            camera_.Reset(); start_attempted_ = false; started_ = false;
            if (FAILED(cleared)) hr = cleared;
        }
        if (mf_started_) {
            const HRESULT ended = MFShutdown();
            if (SUCCEEDED(ended)) mf_started_ = false;
            if (FAILED(ended) && SUCCEEDED(hr)) hr = ended;
        }
        if (!camera_ && !mf_started_ && reservation_.value) {
            CloseHandle(reservation_.value); reservation_.value = nullptr;
        }
        return hr;
    }
};
}
HRESULT reserve_video_output(HANDLE* handle) noexcept {
    if (!handle) return E_POINTER;
    if (*handle) return HRESULT_FROM_WIN32(ERROR_BUSY);
    Handle token;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token.value)) return HRESULT_FROM_WIN32(GetLastError());
    // TokenUser has one bounded SID. No account name, profile data or credential is read.
    alignas(TOKEN_USER) BYTE bytes[sizeof(TOKEN_USER) + SECURITY_MAX_SID_SIZE]{};
    DWORD needed = 0;
    if (!GetTokenInformation(token.value, TokenUser, bytes, sizeof(bytes), &needed)) return HRESULT_FROM_WIN32(GetLastError());
    LPWSTR sid = nullptr;
    if (!ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(bytes)->User.Sid, &sid)) return HRESULT_FROM_WIN32(GetLastError());
    LocalMemory sid_memory{sid};
    wchar_t name[256] = L"Global\\TingXian.Video.Output.";
    wchar_t acl[256] = L"D:P(A;;GA;;;SY)(A;;GA;;;";
    if (wcscat_s(name, sid) || wcscat_s(acl, sid) || wcscat_s(acl, L")")) return E_UNEXPECTED;
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(acl, SDDL_REVISION_1, &descriptor, nullptr))
        return HRESULT_FROM_WIN32(GetLastError());
    LocalMemory security_memory{descriptor};
    SECURITY_ATTRIBUTES security{sizeof(security), descriptor, FALSE};
    HANDLE created = CreateMutexW(&security, FALSE, name);
    const DWORD error = GetLastError();
    if (!created) return HRESULT_FROM_WIN32(error);
    if (error == ERROR_ALREADY_EXISTS) { CloseHandle(created); return HRESULT_FROM_WIN32(ERROR_BUSY); }
    // Presence, not thread ownership, is the reservation. It disappears on process
    // exit and can be closed by the MTA cleanup thread without ReleaseMutex.
    *handle = created;
    return S_OK;
}
CameraProbe probe_virtual_camera() noexcept {
    CameraProbe result;
    CameraApi api;
    result.api_error = api.load();
    result.api_exports_present = SUCCEEDED(result.api_error);
    result.registration_error = registration(result);
    return result;
}
CameraBackend* make_windows_camera_backend() noexcept { return new(std::nothrow) WindowsCameraBackend; }
}
