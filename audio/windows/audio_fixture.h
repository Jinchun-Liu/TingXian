#pragma once
#include "audio_output.h"
// Compiled only into the separately named test DLL, never the native product DLL.
std::unique_ptr<tingxian::audio::Backend> make_fixture_backend();
std::vector<tingxian::audio::Endpoint> fixture_inventory();
HRESULT fixture_step(tingxian::audio::Backend&, UINT32 consumed, UINT32 failure) noexcept;
HRESULT fixture_read(tingxian::audio::Backend&, float*, UINT32 capacity, UINT32* samples) noexcept;
HRESULT fixture_anchor(tingxian::audio::Backend&, UINT64 qpc) noexcept;
