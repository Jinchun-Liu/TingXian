# Scheduled video protocol v2

This opt-in extension carries a corrected image's actual capture time separately
from its planned presentation time. It is an AV-E3 component, not a completed AV
owner or installed meeting output. `open()` and `IFrameReceiver` keep v1 behavior;
`open_timed()` and the new `ITimedFrameReceiver` explicitly select v2. Never convert
v1 capture milliseconds into an assumed common clock or silently downgrade v2.

## Clock and finite scheduling

Times are same-machine QPC converted to 100 ns units. Microsoft's
[FrameServer source contract](https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source#sample-allocation)
requires sample time/duration and describes `MFGetSystemTime` as the QPC wrapper.
An actual MF sample retains a newly selected plan as its PTS. This is a source-side
contract; a meeting client may buffer or rewrite timestamps. Receiver sync is a
separate measurement.

`TimedFrame` is 32 bytes: u64 frame ID, capture QPC, presentation QPC, u32 flags,
u32 zero reserved. Flags 0 requires the full 720p NV12 image; flag 1 is explicit
neutral with no pixels, frame ID or capture time. Transport sequence is separate
from image identity, so multiple plans can refer to one corrected frame without
inflating unique-image counts or refreshing its capture lease.

- Four queued slots and one current slot, fixed NV12 geometry. MF's four output
  samples, the legacy buffer, a bounded send copy and one receive buffer are
  separate allocations. No background send backlog or growing capture history.
- Nonzero epochs strictly increase in one source's lifetime. Sequences and planned
  times strictly increase within an epoch. Image IDs/capture times cannot regress;
  reusing an image ID requires exactly the same original capture time.
- Future plans are limited to 100 ms; capture-to-plan and capture age are at most
  250 ms. Overflow, reserved bits, future captures and expired plans are rejected.
  A plan's slot lasts 333333 ticks (the advertised 30 fps duration).
- Before a plan is due, requests receive the still-valid current image or neutral,
  never the future image. Of multiple due slots, only the latest is selected;
  skipped/expired slots are counted. A plan at/before the last successful sample
  PTS is discarded, not relabelled with the request time.
- A repeated sample advances its MF PTS while diagnostic metadata retains its
  original plan and capture. Its image lease cannot extend beyond capture expiry
  or the current plan slot. New plans for the same image count as repeated images.
- Full queues reject with `HRESULT_FROM_WIN32(ERROR_BUFFER_OVERFLOW)`. Expired
  queued slots retire before admission. Invalid input does not change ordering
  state. Explicit stop clears queued/current bytes and invalidates generation;
  switching v1/v2 requires stopping the active mode.

`SampleTimingAttribute` is a 56-byte `TimedSample`: u32 version/flags and u64 epoch,
sequence, frame ID, capture QPC, original presentation QPC and source-handoff QPC.
The latter is observed immediately before tagging/queuing the MF sample; it is
not a device or remote presentation timestamp. Default neutral has no invented
capture/plan. Bit 2 marks repeated output. Reused v1 samples have this attribute
removed. The 96-byte `TimedMetrics` reports v2 enabled state, local generation,
external epoch, admissions, rejections, queued slots, late,
superseded, cleared, unique, repeated and neutral samples separately from v1.

## Authenticated wire

The [v1 ACL, capability, PID/session, activation and cancellation rules](IPC.md)
are unchanged. The endpoint layout stays 236 bytes, with explicit version 2.
Hello/Ack and every following header must match that version. The 40-byte header
retains magic TXV1 (family marker), but header version distinguishes the protocol.
Begin/End retain kinds 3/5. Kind 4 is forbidden on v2.

Kind 6 is a scheduled frame: header epoch is the external epoch, ID is sequence,
captured is **zero**; payload is `TimedFrame` plus exactly `FrameBytes` media bytes
or no bytes for neutral. Validate exact payload length before reading/allocating.
Unknown kinds, malformed headers and invalid control order close the connection.

After successful Begin, valid frame admission/rejection, or completed End, the
receiver sends kind 7: header version 2, matching epoch/sequence, zero captured,
payload length 4 and an HRESULT payload. This receipt proves admission or mailbox
clear only. It does not prove a sample reached MF, the audio endpoint, FrameServer
or the meeting. A failed I/O after writing is an unknown admission outcome; stop
both routes in the future owner instead of blindly retrying. Stale/invalid/full
frame admissions report failure and remain countable without dropping the pipe.

The publisher serializes a send and receipt under its mutex; per-transfer timeout
is 500 ms. This blocking API must **not** run inside the shared audio renderer's
locked callback. A future joint owner needs finite reservations/dispatch and
paired revocation. A stop receipt cannot retract already queued MF events or
external meeting buffers, nor authorize immediate cross-object restart.

## Evidence and remaining integration

`tingxian-video-timing-test` uses deterministic times and synthetic NV12 to check
boundaries, expiry, accounting, stop/races and callback failure. The real IPC test
spawns its own hidden producer and checks actual MF pixels, metadata, PTS,
admission errors, backpressure, three restarts and version/malformed rejection.
Legacy source and eye-output regressions remain required. No test registers or
opens a camera/mic, plays audio, starts NVIDIA or modifies installed components.

Rust's existing native video endpoint/relay still selects v1. The next step is a
separate v2 host bridge and shared AV owner with bounded asynchronous admission,
paired faults and a persistent drain barrier. Main-window controls, original
microphone capture, installed FrameServer/virtual audio, SDK processing and
meeting-receiver P95 skew remain unverified. A pipe receipt or local preview
cannot mark those gates complete.
