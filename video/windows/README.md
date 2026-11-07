# Windows native video source

This is the native output component for R10E. It implements a COM activation
object, a Media Foundation live source and a single 1280×720 NV12 video stream at
an advertised 30 fps. Optional native eye workers, preview receive and explicit
output relay are separate components; main-window/installed integration is not
complete. The advertised format is not a measured effective-frame-rate result.

The component accepts already processed local frames through an in-process
`IFrameReceiver` or the [authenticated local pipe transport](IPC.md). A producer
helper can send frames from a separate Windows process. MF activation attributes
can configure the transient channel without exposing its capability to source
consumers. Main-window integration and current-chain installed FrameServer checks
remain pending. There is no physical-camera enumeration/opening, audio, external network,
GPU loading, automatic download or registry modification in this DLL.

## Frame and lifecycle behavior

- Default is a neutral NV12 frame (Y=32, UV=128), including after restart.
- `BeginVideo` creates a fresh generation. `SubmitNv12` requires that generation,
  increasing frame IDs, exactly 1,382,400 bytes and a local `GetTickCount64`
  capture timestamp no more than 250 ms old or ahead of the current clock.
- Only the newest pending input is retained. Overwritten unread inputs are counted
  as drops; repeated, unique and neutral samples have separate counters.
- `StopVideo`, source stop and stream pause invalidate the generation and clear
  retained frame bytes. A delayed old submission cannot reactivate it.
- The output process checks age when serving samples. An expired frame disables
  the current generation and produces neutral pixels; only an explicit new begin
  can resume. No worker timer or callback is required for this expiry.
- Generation locking extends through successful MF sample-event handoff. A stop
  cannot return before an old in-flight frame is handed off and later have that
  frame queued. Samples already delivered to a consumer cannot be revoked; actual
  meeting behavior and the PRD's 500 ms/1 s bounds still require device tests.
- Four reusable MF output samples bound the consumer buffer pool. This is separate
  from the single pending input and from any future GPU worker queue (maximum two).
- Counters describe successfully queued source samples, not remote presentation.
  They cannot alone establish effective meeting FPS or end-to-end latency.

## Developer build and no-device test

Use Visual Studio 2022 C++ x64 tools, CMake ≥3.21 and Windows SDK ≥22000. Ordinary
users will not need these tools. Build outside the source tree, on D: locally:

```powershell
cmake -S video/windows -B D:/TingXian-build/video-source -A x64
cmake --build D:/TingXian-build/video-source --config Release --parallel 2
ctest --test-dir D:/TingXian-build/video-source -C Release --output-on-failure
```

The self-test loads the exact DLL by absolute path with restricted DLL search,
creates its class factory directly, requests actual MF samples and compares all
visible NV12 bytes. It never registers or enumerates a device. The synthetic frame
test includes neutral defaults, token/timestamp behavior, generation rejection,
overwrite/drop accounting, stop/restart, expiry, concurrent old submissions,
bounded allocator recovery, shutdown and COM object release.

Hypothesis: these source contracts hold independently of GPU and camera access.
Decision value: decide whether this output component can proceed to authenticated
IPC and separate virtual-device installation. Stop on wrong/stale pixels, an
unexpected frame after invalidation, unbounded allocation, leaked COM objects,
crash or any unexpected media/device side effect. Fix the cause before proceeding.
This test is not an eye-quality, real camera, virtual-device or performance test.

CI workflow `Windows native video source` builds and runs the test on Windows 2022.
The downloadable DLL/test executable is an experimental test artifact, not an
installer or a stable product release. It contains no registration routine.

## Remaining delivery gates

The separate [session controller](SESSION.md) now implements explicit device
creation/cleanup and a read-only prerequisite diagnostic. Its Windows API backend
is compiled, while cancellation and failure recovery use an injected OS boundary
in tests. No device has been registered by these tests and no installer or app
control has been connected yet.

Actual LocalService/FrameServer IPC validation; local eye worker and main controller;
installation/COM registration and session-scoped `MFCreateVirtualCamera` lifetime;
main-window-only controls; output watchdog on actual producer disconnect; real
conference enumeration and pixels; ordinary-user install with normal protection;
measured cadence/latency, GPU coexistence and 30/60 minute meetings. Nothing here
removes those gates or enables the frozen avatar/face/TTS routes.

References: [Microsoft sample](https://github.com/microsoft/Windows-Camera/blob/626f8b19c5f367602f2e89c6b314573d3776c9df/Samples/VirtualCamera/README.md),
[custom source lifecycle](https://learn.microsoft.com/en-us/windows/win32/medfound/writing-a-custom-media-source),
[virtual-camera creation](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-mfcreatevirtualcamera).
