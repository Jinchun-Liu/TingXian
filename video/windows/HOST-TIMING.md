# Native host bridge for scheduled video

`TimedPipeOutput` and Rust `timed_video::NativeTimedVideo` connect a rendered
timeline window to the [explicit v2 receiver](TIMING.md). They start no physical
input, GPU, audio endpoint, registered virtual camera or default output. The host
must load its build-pinned library, obtain explicit output intent, and supply the
transient endpoint only to its authorized native device owner. Legacy preview /
`TxOutput*` stays on ABI/protocol v1.

## Bounded dispatch

The host first opens an idle instance and receives a version-2 endpoint. A
dedicated sender authenticates the source and waits for its Begin receipt. Until
then `reserve` returns S_FALSE / None. Exactly one reservation, staged frame or
unconsumed receipt may exist; another reservation returns backpressure. There is
no latest-frame overwrite and no unbounded FIFO.

`stage` copies at most one fixed NV12 frame under a short mutex and signals the
sender. It performs no pipe wait or join, including on failure. The caller keeps
the original capture QPC and planned presentation QPC from the timeline. A neutral
window has no invented image ID/capture time or pixel payload. The sender performs
the authenticated write and receipt outside the audio timeline callback.

`take` is nonblocking and consumes a receipt once:

| Outcome | Meaning |
| --- | --- |
| Pending | No receipt yet; retain the one outstanding ticket |
| Admitted | Receiver replied exactly S_OK; this is queue admission, not presentation |
| Rejected | Receiver supplied a failing HRESULT; preserve the exact error |
| Unknown | No valid receipt; the image may have been admitted before connection/response failure |
| Cancelled | Intent was revoked; any late admission cannot revive this operation |

Undefined positive HRESULTs are protocol errors, not admission. Unknown/rejected
results terminate the sender. Never blindly retry an unknown result. Cancellation
can supersede an already received but unconsumed admission; it cannot retract
pixels already sent to MF or meeting software.

## ABI, lifetime and ownership

`TingXianVideoOutput.dll` adds separate `TxTimed*` exports, ABI version 2. The old
exports remain version 1. Fixed structures are Ticket 24, TimedFrame 32, Receipt
32 and Status 80 bytes. Exact sizes, generation, serial, reserved fields and state
are checked at the boundaries.

Every native ticket contains a nonreused instance ID within the loaded module's
lifetime plus epoch/serial; counter exhaustion fails instead of wrapping. Native
callers must retire tickets before unloading that module. The Rust ticket also
owns an `Rc` identity, so allocation-address reuse or module reload cannot
redirect a stale ticket to a new instance.
Open/reserve/stage/take/stop/destroy are checked against the creator Windows
thread; Cancel/Status/Event may be observed concurrently within the object's live
lifetime. Rust owners/tickets are !Send/!Sync and expose only borrowed handles.

An auto-reset event wakes the owning native lane on readiness, receipt or stop /
fault. Wait on it together with audio readiness and the joint owner's stop event;
always inspect state afterward. An event is not an admission or permission to
start. One waiter owns it; callers must not close the borrowed handle.

Rust verifies the native host's SHA-256 pin and file/parent leases before loading
by absolute path with system-only dependency lookup. The actual loaded module
path and ABI/exports are checked. There are no settings/webview DLL paths or
serialized endpoint capabilities. Drop cancels and joins the sender before
destroying its native state and releasing the module lease.

## Stop and shared AV integration

Cancel revokes immediately and wakes/cancels pipe I/O without joining. Stop on
the owning lane joins the sender and clears its bytes. The instance is terminal:
it cannot be reopened. Status preserves errors and explicitly keeps
`external_drain_verified=false`; even a local clear cannot prove source event
queues, the virtual microphone or a meeting client have drained.

`crates/video-worker-control/src/joint/` now owns both real bridge adapters,
retains one bounded PCM window pending video admission, checks the audio deadline,
and revokes both on failure. Its durable barrier survives object/process exit;
there is no recovery API until downstream drain/release can be verified. Audio
commit and video admission remain distinct, **non-atomic** operations. Main-window
integration, recovery and actual installed/device/meeting output remain pending.
See `docs/eye-av-owner-validation-2026-11-07.md`; synthetic MF/memory PCM results
must not be reported as synchronized meeting output.

## Tests

`tingxian-timed-output-test` checks the actual C ABI/native transport using synthetic
NV12: one slot, thread/instance binding, receipt replay, known rejection, unknown
I/O, malformed positive receipts and cancellation while a receiver refuses to
consume a large frame. `timed_bridge_*` Rust tests load the actual own DLL and
drive a hidden separate process hosting the actual unregistered MF source. They
check timeline PCM in memory, planned picture/neutral pixels and original/plan
timestamps at MF, pin/intent, instance reuse and cancel. No test captures, plays,
registers a device or proves a meeting client's behavior.
