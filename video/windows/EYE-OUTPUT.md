# Corrected eye output connection

`EyePipeOutput` implements the existing `eye::OutputSink`. It connects a prepared
provider Session to `FramePublisher`, not a physical camera or device registrar.
The SDK-free native library has no vendor, audio, UI, capture or recording API.
No app/installer entry point enables it automatically.

## Ownership and use

1. Construct the output, provider and `eye::Session`; keep the output alive until
   the session's worker lane has drained and joined.
2. Explicitly begin and prepare the Session for 1280x720 packed BGR8. Initial
   neutral callbacks do not grant output permission.
3. After explicit output intent, capture `operation_ticket()` before dispatch;
   call `open_if_current(session_generation, ticket, &endpoint)` on the owner lane.
   A stop between dispatch and admission rejects that ticket. A new lifetime
   requires a strictly newer generation and completed `close()`.
4. Pass the transient endpoint to the authorized local preview/output owner. Do
   not log/persist it or send it through a display webview/command line. The native
   MF activation interface already accepts it; actual device start is separate.
5. Submit input only through Session. Session hands its **completed corrected
   result** to this sink. There is no original-frame fallback. An unavailable
   connection or format mismatch returns failure to Session and stops processing.
6. Stop Session, then drain/join its provider lane; call output `close()` on the
   host owner lane. `neutral()` and `request_stop()` revoke immediately and signal
   pending pipe operations without joining or waiting for the writer lock.

Only one pending BGR result plus one sender-owned result is held by this adapter,
in addition to Session's bounded inference queue and the transport/source buffers.
Replacement increments dropped. The sender converts into one fixed NV12 buffer,
checks original capture age before and after conversion and carries the unchanged
generation/frame ID/capture time onto the pipe. There is no catch-up queue or
timestamp renewal. Stop clears pending results and interrupts accept/write;
already-issued handoffs remain subject to the source's independent 250 ms lease.

Waiting means an explicitly opened endpoint awaiting its one peer. Streaming
means its pipe handshake/begin succeeded. Submitted counts successful pipe
handoffs, **not** MF presentation or meeting receipt. Rejected, dropped, queued,
busy and HRESULT are separate. Transport failure clears pending work and remains
faulted until explicit close/new generation; it never discovers/reconnects peers.
Host integration must observe asynchronous transport faults and stop its provider
lane rather than treating a prior callback acknowledgment as final delivery.

## Color contract

The input is full-range gamma-coded BGR8; the output is limited-range BT.709 NV12,
1280x720, top-down, with 2x2 centered averaged chroma. The media type explicitly
declares BT.709 matrix/primaries/transfer, 16-235 nominal range and progressive
MPEG1 chroma siting. No resizing, gamma re-encoding or original-pixel fallback is
performed. Future capture conversion must produce this declared input color
space. Compare SDK non-eye-region preservation before this lossy 4:2:0 conversion.
The equations and attribute references are in the [validation record](../../docs/eye-output-validation-2026-11-07.md).

## Checks and limits

`tingxian-eye-output-test` checks known color bars, mixed blocks, exact sizes,
bounded pending work, original wire metadata, late connection expiry, three
explicit restarts, blocked-writer cancellation and disconnect faults.
`tingxian-video-ipc-test` also starts its own hidden synthetic eye producer and
checks actual MF pixels in another process after correction, stop, no-face fault
and crash. `tingxian-video-source-test` checks declared color metadata. None of
these registers/starts a device, executes NVIDIA, captures or sends media online.

The native library is a connection component, not installed meeting acceptance.
The worker now also has a native streaming command; its synthetic supervised path
has independent tests. The new `TingXianVideoOutput.dll` C ABI reuses this publisher
for host-side fanout of already corrected NV12 (`corrected_nv12`), bypassing a
second color conversion. The Rust preview owner explicitly enables/disables this
relay with separate intent and pinned loading. See [fanout validation](../../docs/eye-fanout-validation-2026-11-07.md).
Production trust, main-window/device ownership, actual SDK streaming, installed
component update, physical camera, meeting and resource/latency gates remain open.
