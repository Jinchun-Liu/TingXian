# Video worker controller

This is the main-app side of the native worker boundary. Only explicit local
prerequisite inspection exists today; no SDK/camera/output start command is
implemented. The production worker reports `runtime: not_pinned`, never ready.

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
