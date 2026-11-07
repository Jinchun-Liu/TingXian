# Video worker controller

This is the main-app side of the native worker boundary. The shipped Tauri command
still only exposes explicit prerequisite inspection (`runtime: not_pinned`).
Native callers also have separate, explicitly admitted offline and streaming eye
jobs and a corrected-frame preview receiver. These are not camera, virtual-device
or webview start commands, and are not enabled in the current installed app.

The caller supplies a bundled path and a digest embedded at app build time, never
webview input. Before launch, the EXE and parent directories are opened without
delete sharing, reparse paths and hard links are rejected, and SHA-256 is checked
while denying write sharing. Launch uses the absolute EXE, a restricted inherited
handle list, a minimal environment (no app keys/proxy/PATH), no console, and a
suspended process assigned to a kill-on-close job with no descendants before it
can execute. The child receives only a restricted owner process handle, stop
event and bounded report pipe. This is process lifecycle isolation, not a sandbox
against a malicious binary with the user's privileges.

Reserve a `Run` synchronously before dispatch. Stop revokes its lease even if it
has not run yet. Webview admission uses `begin_if_current` with a previously read
generation so an old IPC arriving after Stop cannot reserve a new run.
Exit suspends new reservations before cancellation; cancellation
of exit only reopens admission, never restarts anything. Reports from cancelled
runs are discarded. Crash, invalid/oversized report, timeout and forced cleanup
have distinct results. A cleanup whose process exit could not be confirmed blocks
further starts and clean-exit approval. Production inspection has a five-second
execution deadline; explicit stop gets a one-second cooperative grace, then only
the owned job is terminated. These are controller bounds, not PRD camera-release
or meeting timing claims.

Hypothesis: verified workers can report prerequisites, while cancelled, corrupted,
stale or hung workers cannot claim readiness or leave unrestricted descendants.
Decision value: whether the main app can use this boundary for explicit inspection
and later, separately implemented camera processing. Stop on unexpected execution,
credential inheritance, stale report, unbounded wait or unconfirmed cleanup.

Windows tests require `TINGXIAN_VIDEO_TEST_BIN` (built native fixtures) and
`TINGXIAN_VIDEO_TEST_ROOT` (synthetic evidence directory, D: on the target PC).
They execute the real production inspection and separate synthetic fixtures; no
physical camera, virtual device, GPU/SDK, audio or network is started. Keep all
fixture directories and failures. A passing controller suite does not verify Tauri
webview authority, packaged installation, eye inference or meeting reception.

## Native preview receive lane

After reserving and dispatching a stream, `StreamFeed::open_preview` consumes its
single-use output endpoint. Call it and `NativePreview::poll` on a dedicated native
media lane. The endpoint must not enter webview IPC, logs or preferences. The pipe
is opened without inheritable handles and with identification-only security QoS;
the receiver verifies server PID/session and the capability handshake. It creates
no camera device and loads no vendor runtime itself.

`poll` accepts at most one fixed-size 720p limited-range BT.709 NV12 frame at a time.
Only one latest completed frame plus the current incoming payload is retained.
The host generation is bound to the stream's ready message and endpoint; the
worker-local epoch is checked separately and retained as `worker_generation`.
Frame ID and original GetTickCount64 timestamp are preserved. Invalid packets fail
closed, with absolute 500 ms packet I/O deadlines; idle polling is not a fault.
Pending I/O is cancelled and its completion observed before releasing its buffer.
This is a bounded native transport policy, not a hard realtime OS guarantee.

`with_current` passes a borrowed frame or `None` for a neutral placeholder. The
handoff holds the stream lease so controller Stop cannot return and then admit a
delayed old-frame callback. The callback must be short, synchronous and must not
call back into Controller/StreamFeed or enqueue the borrowed frame for later
presentation. It must not be used as a blocking UI or GPU operation. Stopped,
future-dated and older-than-250-ms frames are never handed off; there is no raw
fallback. Status contains only counts/phase/reason, not image or endpoint data.

Stop the owning controller before dropping the receiver during normal shutdown.
Dropping the receiver alone disconnects its pipe and faults the worker output;
it is not a success acknowledgment. No detached receive thread is created here.
Preview consumes the worker's sole endpoint. A separately enabled relay now
provides a different endpoint for the native output owner, described below.
Main-window rendering/device ownership, real SDK streaming, installation and
meeting reception remain unverified.

## Explicit output relay

`NativePreview::enable_output` requires the current output intent and a native
build-pinned `PinnedOutputLibrary`. It loads our `TingXianVideoOutput.dll` only for
this explicit operation. Missing or mismatched binaries leave local preview
usable. File/directory locks and SHA-256 verification precede full-path loading;
dependencies resolve from System32, and the returned module path/ABI are checked.
Keep these native pins out of webview input/settings. No vendor DLL is involved.

The DLL exposes a small C ABI over the existing C++ `EyePipeOutput` publisher,
including its logon-SID/LocalService DACL and authenticated transport. Completed
NV12 is relayed without reconversion, resizing or timestamp renewal. One latest
queued result plus the sender's current result bounds the relay; there is no
delivery acknowledgment implied by enqueue or pipe submission. Existing preview
pixels are not replayed when output is enabled: only subsequently received frames
may be submitted. Output has its own epoch; host/worker identities remain on the
local preview side, while original frame ID/time survive forwarding.

`disable_output` retires that relay and advances its intent, leaving preview/input
alive. A queued old enable is rejected. Full controller Stop independently cancels
the relay through a weak native revocation lease, even before another preview poll
or renderer call. Preview fault/drop also retires the relay. The owner must continue
polling to observe asynchronous relay errors and fail the processing stream closed.
Do not interpret Waiting/Forwarding as a device, display or remote meeting state.

`video/windows/output_bridge.h` documents the ABI and thread contract. Open,
Submit, Close and Destroy are serialized on the native lane; only Cancel/Status
are callable concurrently. DLL lifetime extends through sender retirement and
all outstanding revocation references. It is not currently packaged in the CPU
installer and no Tauri start API exposes it yet.
