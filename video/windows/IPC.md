# Local video pipe protocol v1

`FramePublisher` belongs in the optional user-mode eye worker. `PipeReceiver`
belongs in the media source. They now exchange actual NV12 bytes across Windows
processes. The application/worker and virtual-device installer are still absent;
no camera or audio is opened by this transport.

## Identity and activation

Activation validates and privately retains the endpoint without connecting.
FrameServer may create temporary sources solely to inspect formats; these probes
must not consume the producer's single connection. The first actual source Start
attempts that connection once, after validating the selected stream and allocator.
It then erases the cached endpoint. Shutdown, StopVideo and DisconnectPipe also
erase an unconsumed endpoint, so a delayed Start cannot undo an explicit stop.
There is no automatic reconnect after a transport fault or stream teardown.

An explicit publisher `open()` creates one `\\.\pipe\TingXian.Video.<32 hex>`
instance with `FILE_FLAG_FIRST_PIPE_INSTANCE` and `PIPE_REJECT_REMOTE_CLIENTS`.
The protected DACL allows only the producer's logon SID and LocalService. The
LocalService ACE excludes `FILE_CREATE_PIPE_INSTANCE`; no Everyone or Anonymous
ACE is installed. A random 256-bit capability is generated for each endpoint.

The receiver validates the exact local name, kernel-reported producer PID and
session, then sends a capability handshake with `SECURITY_IDENTIFICATION`. The
producer checks the capability and the client's impersonated logon SID or
LocalService identity before sending any frame. Authentication failures cannot
enable output. Impersonation failure rejects the request; failure to revert is
fatal to that worker rather than continuing under the wrong identity.

Endpoint data travels through `IFrameTransport::ConnectPipe` or a transient
`PipeEndpointAttribute` blob on `IMFActivate`. The latter is the integration seam
for a future explicitly started `IMFVirtualCamera`. The source strips the blob
from `GetSourceAttributes` so consumers do not receive the capability. Connection
failure leaves a neutral source with queryable fault status. There is no automatic
reconnect, endpoint discovery or camera selection. Do not persist endpoint data in
preferences, session files or logs, and do not pass capabilities on command lines.

These controls separate local sessions and reject an unconfigured peer. They are
not a sandbox against malicious processes already able to inspect the same user's
memory, or against administrators. LocalService authorization is implemented but
actual FrameServer/Session 0 access still requires installation testing; two user
processes cannot prove it. No external TCP/HTTP listener is used.

## Wire and cancellation

The wire header is a fixed 40-byte little-endian structure: magic `TXV1`, 16-bit
version and kind, 32-bit payload length and zero reserved field, then 64-bit epoch,
frame ID and capture milliseconds (`GetTickCount64`, same machine). Kinds are
Hello=1, Ack=2, Begin=3, Frame=4 and End=5. Hello carries the 32-byte capability;
other control messages have no payload. Frames must be exactly 1,382,400 bytes
(1280×720 NV12). Unknown headers and oversized payload declarations fail before
allocation or payload reads. The receiver allocates one fixed frame buffer.

Begin requires a strictly newer nonzero wire epoch. It maps to a new source
generation. End invalidates the active generation; later old-epoch frames are
read within the fixed size, discarded and counted. Invalid frame IDs/timestamps
are also discarded. No message can revive a generation invalidated by the output
watchdog without an explicit new Begin/reconnect as appropriate.

Pipe I/O is overlapped, with a cancellation event and 500 ms per-transfer deadline.
On cancellation the code retires the outstanding OVERLAPPED operation before
freeing its storage. The initial publisher accept waits until a client connects
or the owner cancels; a meeting opening later is not itself a failure or permission.
The publisher serializes writes and has no unbounded send queue. The 128 KiB pipe
buffer, one receive buffer and one latest-frame mailbox are separate from the
four downstream MF output samples. This copying path still needs throughput and
resource measurements before it can be used for daily meetings.

EOF, producer exit, malformed input or idle read timeout closes the receiver pipe
and invalidates cached pixels. The existing 250 ms output lease also works when
the receiver thread is stalled. User StopVideo, source Stop and Shutdown cancel
and join the receiver before returning. Already handed-off samples cannot be
revoked; actual meeting stop behavior remains a separate gate.

## Native checks

Hypothesis: the real two-process path preserves pixels and rejects invalid peers,
stale epochs and malformed lengths, while stop/disconnect produces neutral output.
Decision value: whether the transport is ready for FrameServer installation and
eye-worker integration. Stop on wrong/stale pixels, identity acceptance, leaked
capability, blocked cancellation, crash or unbounded allocation. Tests use synthetic
NV12 bytes only; green checks are not eye inference or meeting acceptance.

`tingxian-video-ipc-test` starts only its own hidden producer children. It passes
control/endpoint data through explicitly whitelisted inherited anonymous-pipe
handles, not command-line secrets. It verifies the live pipe DACL and actual MF
sample bytes, abrupt process exit, stop/new epochs, cancellation under 500 ms in
the test case, wrong PID/session/capability, nonlocal paths, oversized input,
explicit recovery and activation-attribute redaction. All children and handles are
bounded by the test; no device registration, account switching or elevated service
is used. An injected constructor failure verifies exact-child cleanup even before
the test fixture finishes initialization. The older 15-group native media-source
test remains required.

References: [pipe access rights](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights),
[kernel producer PID](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getnamedpipeserverprocessid),
[client identification](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-impersonatenamedpipeclient),
[overlapped cancellation](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-cancelioex).
