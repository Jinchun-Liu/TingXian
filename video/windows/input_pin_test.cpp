#include <windows.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <iostream>
int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;
    HMODULE library = LoadLibraryExW(argv[1], nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!library) return 3;
    using Create = HRESULT (WINAPI*)(IMFSourceReaderCallback**);
    const auto create = reinterpret_cast<Create>(GetProcAddress(library, "TxCameraFixtureCallback"));
    if (!create) return 4;
    IMFSourceReaderCallback* callback = nullptr;
    if (FAILED(create(&callback)) || !callback) return 5;
    // Drop the caller's module reference while the callback still exists. The
    // pin must retain code even before its final Release stack returns.
    if (!FreeLibrary(library)) return 6;
    HMODULE mapped = nullptr;
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(create), &mapped) || mapped != library) return 7;
    if (FAILED(callback->OnReadSample(E_ABORT, 0, 0, 0, nullptr))) return 8;
    if (callback->Release() != 0) return 9;
    mapped = nullptr;
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(create), &mapped) || mapped != library) return 10;
    std::cout << "PASS actual DLL remains mapped through late callback and final Release; no device opened\n";
    return 0;
}
