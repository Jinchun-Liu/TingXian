# Virtual-camera session controller

`camera_session.*` implements the explicit output lifecycle. `camera_windows.cpp`
supplies the actual Windows backend; it is compiled into a separate static library,
not automatically loaded by the base app. The optional MSI has passed CI and
target-machine installation/COM checks; main-window/Tauri integration remains.

## Runtime contract

- Construction and restoration do nothing. A dedicated MTA worker calls `start`
  only for a new user action with a freshly opened producer endpoint. The display
  window must never receive this controller, endpoint or output lease.
- The backend checks the 64-bit HKLM source registration, rejects a conflicting
  HKCU registration, and reserves one output owner per user across login sessions.
  Reservation uses a protected named kernel object with user/SYSTEM access and
  process lifetime; it neither writes persistent state nor takes over an existing
  owner. This coordination is not a hostile-process security boundary; a squatted
  name is an explicit failure, never a reason to weaken permissions.
- `MFCreateVirtualCamera` uses only SoftwareCameraSource, Session, CurrentUser,
  the project's own CLSID and the name `听弦视频`. There is no AllUsers/System mode,
  physical-camera wrapping, login task, automatic restoration or registry write.
- The transient pipe configuration is passed through the MF attribute store before
  `Start`. Successful Start means the device API accepted registration. It does
  not mean source activation, pipe connection, corrected frames or remote delivery.
- `request_stop` can run while an OS operation is pending: first invalidate the
  operation ticket and revoke the producer lease. `stop` then serializes OS cleanup.
  A late successful Start is closed, not promoted to an active output session.
- The lease owner must stop submitting frames, close `FramePublisher`, and cancel
  its capture worker. Closing the producer also causes the output source to become
  neutral independently of success/failure of Windows device cleanup. No source
  registration/controller method starts physical capture or audio itself.
- Always attempt Shutdown after a Stop failure. Keep the original operation error
  separate from cleanup errors. A failed Shutdown keeps ownership for explicit
  retry and blocks another start. The owner must inspect explicit stop results;
  destructors provide a best-effort backstop, not proof of successful cleanup.
- The Windows backend requires the caller's MTA to remain alive through cleanup.
  The controller does not forcibly terminate Windows calls. A hung OS operation
  can delay device cleanup even though producer authority was already revoked;
  actual 500 ms / 1 s PRD stop bounds require installed-device/capture evidence.

## Read-only prerequisite diagnostic

`tingxian-video-device-probe.exe --probe` reads the exact registration and checks
for two exports in the System32 DLL with restricted library search. It does not
call MFCreateVirtualCamera, MFIsVirtualCameraTypeSupported, enumerate a camera,
request privacy consent, initialize MF or register anything. No arguments produce
usage and exit 2; there is no start/install command in this diagnostic.

Exit 0 means inspection finished, not runtime readiness. JSON separates API export
presence, machine registration, user conflict and exact HRESULTs. It never emits a
DLL path, account SID, pipe capability or camera image. Registration presence is
not a signature/hash/ACL audit of the installed DLL, SDK availability or permission
approval. The future optional installer must deploy verified files to a protected,
service-readable location; CurrentUser does not replace that installation step.

## Decision-driven validation

Hypothesis: explicit device intent cannot survive cancellation, initialization
failure or owner teardown; producer revocation works even if OS Stop fails.
Decision value: determine whether the lifecycle can be used by the main controller
and later installed-device tests. Stop on unintended Start, lost primary errors,
replacement of a handle whose shutdown failed, stale output after revocation,
wrong cleanup order or an unresolved race. Do not reduce assertions to pass.

`tingxian-video-session-test` injects the OS lifecycle boundary for failure and race
tests. It also uses the real named-kernel reservation and actual publisher/receiver
pipe with synthetic pixels to test independent revocation. The production backend
is exercised only for rejection before device access (COM missing/STA). These are
not real Windows device-registration or FrameServer/LocalService tests. The existing
source and IPC tests continue to verify the actual DLL/COM/MF samples separately.

Next gates: target component uninstall/reinstall, actual Session and
CurrentUser lifecycle through FrameServer, application/worker authority and stop
integration, SDK loading and eye processing, UI and installation, camera/meeting
and performance acceptance. Camera and audio remain off until authorized.

## Installed-device synthetic verification

`tingxian-video-device-synthetic-test --describe` has no media/device side effects
and is the only mode invoked by ordinary CTest. `--run-synthetic-device` is an
explicit, separately authorized Windows 11 check after installing the verified
component. It creates a Session / CurrentUser virtual camera via the production
backend, obtains only that instance's shared preview via `GetMediaSource`, and
reads generated gray NV12 frames through an asynchronous Media Foundation reader.
It never selects/opens a physical camera, opens audio, records images, calls an
eye SDK or changes privacy settings. Windows may still require camera API consent;
permission denial is a failure, never a reason to change settings automatically.

Hypothesis: the installed source can be activated through FrameServer, receive the
authenticated producer's synthetic pixels, become neutral after producer closure,
and disappear after shutdown. Decision value: whether the OS/IPC implementation
can support application integration. Stop on the first API, pixel, timeout or
cleanup failure; preserve the stage and HRESULT. Three cycles check actual restart
and reservation release only after the preceding cycle passes. Each sample wait is
bounded to five seconds; OS device calls may still block and need investigation.
The program logs stages, counts and HRESULTs, never the pipe capability, device
identifier or pixels. Enumeration reads metadata only to match its own exact
symbolic link, before and after shutdown; it never activates an enumerated source.

The new backend preview/link getters require successful Start and must be called
on its owning MTA, serialized with Stop/Shutdown. The preview is a shared client,
not an exclusive physical capture source. A passing test would establish this
installed synthetic path, not real camera permission/quality, effective meeting
FPS, end-to-end synchronization, third-party application selection or eye inference.

## Official references checked 2026-11-06

- [Creation and supported lifetime/access](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-mfcreatevirtualcamera)
- [Start and activation events](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-imfvirtualcamera-start)
- [Shutdown removes session devices](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-imfvirtualcamera-shutdown)
- [Microsoft activation attribute example](https://github.com/microsoft/Windows-Camera/blob/626f8b19c5f367602f2e89c6b314573d3776c9df/Samples/VirtualCamera/VirtualCameraManager_WinRT/VirtualCameraRegistrar.cpp)
- [Kernel object namespaces](https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces)
- [CreateMutex and object lifetime](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw)

This is implementation documentation, not a statement that device or release gates
have passed. Test binaries and the library are experimental components.
# Native owner bridge (AV-E7b, 2026-11-08)

`TingXianVideoOutput.dll` now exposes `TxDeviceAbi=1` and the fixed
create/start/cancel/status/stop/destroy ABI in `device_bridge.h`. It reuses the
existing CameraSession/WindowsCameraBackend. Construction does not initialize
COM, activate or register a device. Start is one-shot, explicit intent and v2
only; create-time cancellation ticket rejects stop-before-queued-start. The
caller supplies a dedicated MTA, separate from the continuous audio owner lane.

The native revocation callback must revoke both media routes without waiting for
OS device calls. It runs at most once; callback code/context live until successful
destroy. Start/stop/destroy require the creating thread. Cancel/status can run
concurrently with owner operations, but the caller must serialize them against
destruction. Destroy failure retains ownership and allows an explicit cleanup
retry. No other instance may bypass uncertain cleanup or external drain.

The Rust binding is `timed_video::device::NativeDevice`. It reuses the pinned,
locked DLL loader, takes an exact `Arc<host::Input>` (not a broad Host stop), and
obtains that Input's native endpoint itself. Revocation cannot stop a newer
generation. Cross-thread tokens become inert before freeing the native object;
failed Drop retains the complete library/object/context until process exit.

This is a native integration API, **not** a main-window start command or proof
that the installed COM media source is trusted/compatible. Full scheduling,
source binary trust, device compatibility and receiver drain remain gates.
CI uses injected backends and inert/rejected production-DLL calls only.

The dedicated Rust `timed_video::device::host::Host` now owns that MTA and an
independent cancellation monitor. It follows the exact media Input/source lease;
slow Windows Start cannot block the media lane. Failed close retains the live
apartment and native owner until a repeated explicit stop requests cleanup retry.
Main-window stop, suspend/quiesce/resume and inspection admission include this
owner. No UI media start is enabled yet; see the AV-E7c acceptance record.
