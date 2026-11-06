#include "ar087_provider.h"
#include "ar087_bridge.h"
#include <bcrypt.h>
#include <array>
#include <filesystem>
#include <iostream>
#include <set>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>
namespace tingxian::eye {
namespace {
struct RuntimePin { const wchar_t* path; uint64_t size; const char* sha256; };
// Defines RuntimeRoot (absolute protected official installation directory) and
// RuntimePins. Never generated from an untrusted package at application launch.
#include "ar087_local_pins.h"
namespace fs = std::filesystem;
void need(bool ok) { if (!ok) throw std::runtime_error("runtime_integrity_or_load"); }
struct Runtime {
    std::vector<HANDLE> handles;
    std::vector<HMODULE> modules;
    DLL_DIRECTORY_COOKIE directory = nullptr;
    ~Runtime() {
        for (auto i = modules.rbegin(); i != modules.rend(); ++i) FreeLibrary(*i);
        if (directory) RemoveDllDirectory(directory);
        for (auto h : handles) CloseHandle(h);
    }
    HANDLE lock(const fs::path& path, bool dir) {
        HANDLE h = CreateFileW(path.c_str(), dir ? FILE_READ_ATTRIBUTES : GENERIC_READ,
            FILE_SHARE_READ | (dir ? FILE_SHARE_WRITE : 0), nullptr, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | (dir ? FILE_FLAG_BACKUP_SEMANTICS : FILE_FLAG_SEQUENTIAL_SCAN), nullptr);
        need(h != INVALID_HANDLE_VALUE);
        handles.push_back(h);
        BY_HANDLE_FILE_INFORMATION info{};
        need(GetFileInformationByHandle(h, &info) != 0);
        need(!(info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT));
        need(bool(info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) == dir);
        if (!dir) need(info.nNumberOfLinks == 1);
        return h;
    }
    static std::string hash(HANDLE file) {
        BCRYPT_ALG_HANDLE alg = nullptr;
        BCRYPT_HASH_HANDLE hash = nullptr;
        need(BCryptOpenAlgorithmProvider(&alg, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0);
        const auto result = BCryptCreateHash(alg, &hash, nullptr, 0, nullptr, 0, 0);
        if (result < 0) { BCryptCloseAlgorithmProvider(alg, 0); need(false); }
        std::array<unsigned char, 65536> buffer;
        std::array<unsigned char, 32> digest{};
        bool ok = true; DWORD read = 0;
        for (;;) {
            if (!ReadFile(file, buffer.data(), DWORD(buffer.size()), &read, nullptr)) { ok = false; break; }
            if (!read) break;
            if (BCryptHashData(hash, buffer.data(), read, 0) < 0) { ok = false; break; }
        }
        if (ok) ok = BCryptFinishHash(hash, digest.data(), DWORD(digest.size()), 0) >= 0;
        BCryptDestroyHash(hash); BCryptCloseAlgorithmProvider(alg, 0);
        need(ok);
        std::string hex;
        for (auto byte : digest) { hex += "0123456789abcdef"[byte >> 4]; hex += "0123456789abcdef"[byte & 15]; }
        return hex;
    }
    void verify() {
        const fs::path root(RuntimeRoot);
        need(root.is_absolute() && root.root_name().wstring().size() == 2);
        fs::path current = root.root_path();
        for (const auto& part : root.relative_path()) { current /= part; lock(current, true); }
        std::set<fs::path> allowed, dirs;
        for (const auto& pin : RuntimePins) {
            const fs::path relative(pin.path);
            need(relative.is_relative() && relative.lexically_normal() == relative);
            for (const auto& part : relative) need(part != L".." && part.wstring().find(L':') == std::wstring::npos);
            need(allowed.insert(relative).second);
            for (auto parent = relative.parent_path(); !parent.empty(); parent = parent.parent_path()) dirs.insert(parent);
        }
        for (const auto& dir : dirs) lock(root / dir, true);
        // Complete official tree, not a runtime search in arbitrary directories.
        for (const auto& entry : fs::recursive_directory_iterator(root)) {
            const auto relative = entry.path().lexically_relative(root);
            const auto attrs = GetFileAttributesW(entry.path().c_str());
            need(attrs != INVALID_FILE_ATTRIBUTES && !(attrs & FILE_ATTRIBUTE_REPARSE_POINT));
            need(entry.is_directory() ? dirs.count(relative) == 1 : allowed.count(relative) == 1);
        }
        for (const auto& pin : RuntimePins) {
            const auto h = lock(root / pin.path, false);
            LARGE_INTEGER size{};
            need(GetFileSizeEx(h, &size) && uint64_t(size.QuadPart) == pin.size);
            need(hash(h) == pin.sha256);
        }
        need(SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS) != 0);
        directory = AddDllDirectory(root.c_str()); need(directory != nullptr);
    }
    HMODULE load(const wchar_t* name) {
        const auto path = fs::path(RuntimeRoot) / name;
        const auto module = LoadLibraryExW(path.c_str(), nullptr,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32 | LOAD_LIBRARY_SEARCH_USER_DIRS);
        need(module != nullptr); modules.push_back(module);
        std::array<wchar_t, 32768> actual{};
        const auto count = GetModuleFileNameW(module, actual.data(), DWORD(actual.size()));
        need(count && count < actual.size() && _wcsicmp(actual.data(), path.c_str()) == 0);
        return module;
    }
};
class Ar087 final : public EyeCorrectionProvider {
    std::unique_ptr<Runtime> runtime_;
    EyeAr087* context_ = nullptr;
    Format format_;
    bool cleanup_uncertain_ = false;
public:
    ~Ar087() override {
        if (!stop()) (void)runtime_.release(); // retain modules/locks until process exit
    }
    Capabilities capabilities() const noexcept override { return {"nvidia_ar_sdk", "0.8.7.0_ada", true}; }
    Error prepare(Format format) override {
        if (!format.bytes() || context_ || cleanup_uncertain_) return Error::unavailable;
        runtime_ = std::make_unique<Runtime>();
        runtime_->verify();
        auto ar = runtime_->load(L"nvARPose.dll"), cv = runtime_->load(L"NVCVImage.dll");
        auto cuda = runtime_->load(L"cudart64_12.dll");
        const auto models = (fs::path(RuntimeRoot) / L"models").u8string();
        format_ = format;
        const int status = eye_ar087_open(ar, cv, cuda, models.c_str(), format.width, format.height, &context_);
        if (status) std::cerr << "ar087_prepare_status=" << status << '\n';
        return status == 0 ? Error::none : Error::unavailable;
    }
    Result process(const Frame& frame) override {
        if (!context_ || frame.bgr.size() != format_.bytes() || cleanup_uncertain_) return {Error::invalid_frame, {}};
        std::vector<uint8_t> output(format_.bytes(), 0);
        const auto status = eye_ar087_process(context_, frame.bgr.data(), output.data());
        if (status < 0) std::cerr << "ar087_process_status=" << status << '\n';
        if (status != 0) return {status == 1 ? Error::no_face : Error::inference, {}};
        return {Error::none, std::move(output)};
    }
    bool stop() noexcept override {
        if (cleanup_uncertain_) return false;
        const auto status = eye_ar087_close(context_); context_ = nullptr;
        cleanup_uncertain_ = status != 0;
        if (!cleanup_uncertain_) runtime_.reset();
        return !cleanup_uncertain_;
    }
};
}
std::unique_ptr<EyeCorrectionProvider> make_ar087_provider() { return std::make_unique<Ar087>(); }
}
