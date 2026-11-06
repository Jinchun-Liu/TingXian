#pragma once
// Test target ONLY. Production worker does not compile this header.
#include "provider.h"
#include <windows.h>
#include <array>
#include <filesystem>
#include <memory>
#include <string>
namespace tingxian::eye {
class SyntheticWorkerProvider final : public EyeCorrectionProvider {
    HANDLE stop_;
    std::wstring mode_;
    static void mark(const wchar_t* name) {
        auto marker = CreateFileW(name, GENERIC_WRITE, FILE_SHARE_READ, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (marker != INVALID_HANDLE_VALUE) {
            const auto pid = std::to_string(GetCurrentProcessId()); DWORD written = 0;
            WriteFile(marker, pid.data(), DWORD(pid.size()), &written, nullptr); CloseHandle(marker);
        }
    }
public:
    explicit SyntheticWorkerProvider(HANDLE stop) : stop_(stop) {
        std::array<wchar_t, 32768> path{};
        GetModuleFileNameW(nullptr, path.data(), DWORD(path.size()));
        mode_ = std::filesystem::path(path.data()).filename().wstring();
        mark(L"launched");
        if (GetEnvironmentVariableW(L"TINGXIAN_TEST_SECRET", nullptr, 0)) ExitProcess(72);
    }
    Capabilities capabilities() const noexcept override { return {"synthetic", "1", true}; }
    Error prepare(Format) override {
        mark(L"preparing");
        if (mode_ == L"eye-hang.exe") Sleep(INFINITE);
        if (mode_ == L"eye-owner.exe") Sleep(10000); // finite backstop for failed owner-death tests
        if (mode_ == L"eye-wait.exe") WaitForSingleObject(stop_, INFINITE);
        return Error::none;
    }
    Result process(const Frame& frame) override {
        mark(L"processing");
        if (mode_ == L"eye-crash.exe") ExitProcess(71);
        if (mode_ == L"eye-fail.exe") return {Error::inference, frame.bgr};
        if (mode_ == L"eye-slow.exe") WaitForSingleObject(stop_, INFINITE);
        return {Error::none, std::vector<uint8_t>(frame.bgr.size(), 200)};
    }
    bool stop() noexcept override { return mode_ != L"eye-cleanup.exe"; }
};
inline std::unique_ptr<EyeCorrectionProvider> make_synthetic_worker_provider(HANDLE stop) {
    return std::make_unique<SyntheticWorkerProvider>(stop);
}
}
