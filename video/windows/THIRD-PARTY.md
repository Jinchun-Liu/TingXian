# Native Windows video source references

The Media Foundation source/stream, event, activation and allocator contracts were
implemented with reference to Microsoft's Windows-Camera VirtualCamera sample:

- Repository: https://github.com/microsoft/Windows-Camera
- Reviewed commit: `626f8b19c5f367602f2e89c6b314573d3776c9df`
- Relevant directory: `Samples/VirtualCamera/VirtualCameraMediaSource`
- License: MIT, reproduced in `LICENSE-Microsoft.txt`.

TingXian uses its own CLSID, WRL implementation, single NV12 format, bounded frame
mailbox, neutral fallback, generation checks and output-side expiry. The sample's
physical-camera wrappers, device activation, configuration app, RGB color controls,
registration and installer code are not included.

Build dependencies are the Microsoft C++ compiler, CMake and Windows SDK. No
NVIDIA SDK, model, Python, C++/WinRT NuGet package or third-party GPU runtime is
downloaded by this build. The component links the MSVC runtime statically and uses
Windows Media Foundation system DLLs. This does not grant a right to redistribute
any separate NVIDIA component or Microsoft development toolchain.
