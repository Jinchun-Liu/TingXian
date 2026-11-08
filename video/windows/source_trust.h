#pragma once
#include <windows.h>

namespace tingxian::video {
// Read-only byte pin and lifetime lease. Never loads/activates the source DLL.
// Only accepts the fixed system installation, not caller-selected search paths.
class SourceLease {
    HANDLE handles_[4]{};
public:
    SourceLease() = default;
    ~SourceLease() { release(); }
    SourceLease(const SourceLease&) = delete;
    SourceLease& operator=(const SourceLease&) = delete;
    HRESULT acquire(const wchar_t* registered_path) noexcept;
    void release() noexcept;
};
// Internal read-only checks shared with synthetic tests; not exported by a DLL.
HRESULT check_source_digest(HANDLE file, const char* expected) noexcept;
bool protected_source_descriptor(PSECURITY_DESCRIPTOR descriptor, bool registry) noexcept;
HRESULT check_source_security(HANDLE object, bool registry) noexcept;
}
