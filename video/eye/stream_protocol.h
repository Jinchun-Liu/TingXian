#pragma once
#include "../windows/frame_transport.h"

namespace tingxian::eye::stream {
// Inherited, local byte pipes only. Never put an endpoint/capability in argv,
// logs, a serialized UI snapshot or persisted settings.
inline constexpr uint32_t InputMagic = 0x31495854; // TXI1
inline constexpr uint32_t ReadyMagic = 0x52455854; // TXER
inline constexpr uint32_t BgrBytes = video::Width * video::Height * 3;
inline constexpr DWORD IdleTimeoutMs = 5000, PayloadTimeoutMs = 1000;
enum : uint16_t { Frame = 1, Finish = 2 };
struct InputHeader {
    uint32_t magic = InputMagic;
    uint16_t version = 1, kind = Frame;
    uint32_t bytes = BgrBytes, reserved = 0;
    uint64_t generation = 0, id = 0, captured_ms = 0;
};
struct ReadyHeader {
    uint32_t magic = ReadyMagic;
    uint16_t version = 1, kind = 1;
    uint32_t bytes = sizeof(video::VideoPipeEndpoint), reserved = 0;
    uint64_t generation = 0;
};
struct Ready { ReadyHeader header; video::VideoPipeEndpoint endpoint; };
static_assert(sizeof(InputHeader) == 40 && sizeof(ReadyHeader) == 24);
static_assert(sizeof(video::VideoPipeEndpoint) == 236);
static_assert(sizeof(Ready) == 264); // final 4 bytes of alignment are NOT sent
inline constexpr DWORD ReadyBytes = sizeof(ReadyHeader) + sizeof(video::VideoPipeEndpoint);
bool valid_input(const InputHeader& h, uint64_t generation) noexcept;
int run(int argc, wchar_t** argv);
}
