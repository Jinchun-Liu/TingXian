# Worker owner lifetime (integration boundary)

`WorkerOwnerGuard` is the native worker's one-shot owner/stop boundary. It is not
yet called by Tauri or packaged as an eye worker; the SDK adapter and actual app
commands remain to be implemented. Constructing it does not open a camera or
create a device. Existing ordinary-user app behavior is unchanged.

The future main-window supervisor must create an anonymous **manual-reset** event
for each explicit start, duplicate its own real process handle with only
`SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION`, and pass that handle and an
event handle with only `SYNCHRONIZE` through `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
It must never give the worker a stop-event reset right, reuse a stopped event,
pass all inheritable app handles, or accept handles/PIDs from a webview. The
display window gets no such authority. Worker launch must verify the executable
and use an explicit absolute path; this library is not a launcher or sandbox.

The worker arms a guard before attempting one start on its dedicated MTA. The
guard duplicates handles as non-inheritable, checks for an already-dead owner or
already-signaled stop, and monitors both without polling. Owner exit, explicit
stop, guard closure or a wait error revokes the existing `CameraSession` producer
lease. The watcher does not call COM device APIs. The owning MTA must then call
`session.stop()` and inspect the cleanup result. Revocation is independent of
slow OS Start/Stop, but OS cleanup can still block; no hard cleanup-time claim is
made here. A later supervisor needs a bounded hung-worker policy as well.

The start cancellation ticket is captured **before** arming the watcher and is
carried into `start_if_current`. Capturing it only when a delayed start task runs
would lose a stop that already occurred. A guard is never rearmed or restarted;
restoration of preferences cannot restore its authority. Destructor cleanup is a
backstop and does not constitute proof of successful OS shutdown.

## Decision and validation

Hypothesis: an owner that has exited, a stopped lease or a queued old start can
never enable/re-enable output; a late successful OS Start is cleaned up after
independent producer revocation. Decision value: whether this owner boundary can
be used in the future main-app/worker supervisor. Stop at any unexpected Start,
unrevoked output, surviving test child, lost cancellation or false cleanup result.
Affected artifact: the native session library, not an eye-correction release.

`tingxian-video-worker-owner-test` creates two hidden, ordinary-user test processes
with explicit inherited-handle lists. One acts as the controller owner; the other
executes the real guard. The device backend is injected and opens no camera. Six
process cases cover stop, owner exit, already-stopped/dead owner, delayed start,
and stop during a blocked OS Start. Two local cases cover invalid ownership and
queued cancellation. Test cleanup may terminate **only its own exact spawned
process handle** after failure/timeout; that remains a failure. No broad process
termination, physical capture, SDK load, device registration or network is used.

These tests do not prove Tauri authority, main-app exit/archive integration,
physical capture release, installed-device owner-death removal, SDK/GPU operation,
performance or meeting reception. Those remain separate gates.

References checked 2026-11-06: [handle allowlist](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute),
[hidden process creation](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags).
