# Windows original-voice output adapter (experimental)

This optional native library now has a fixed C ABI and a Rust owner-thread wrapper,
tested with the Rust timeline through a separate in-memory backend DLL. The main
window, paired virtual camera, installer and meeting software are not connected.
It creates no virtual microphone.
It has no capture API and does not wait for ASR, synthesize speech or change system
defaults. The app's existing installed processes and user data are untouched.

## Native boundary

- `TxAudioInspectPair` is an additive ABI 1 metadata-only export for an idle
  owner. It never calls backend prepare/start or consumes the one preparation
  attempt. The Rust joint `Candidate` separates this inspection from explicit
  activation and rechecks inventory before durably arming the exposure barrier.
  Invalid selections can be corrected without poisoning an unexposed profile;
  a previous uncertain-drain record is never cleared by inspection. Actual
  prepare still revalidates. New hosts reject DLLs missing this export.
- `enumerate_endpoints` reads MMDevice properties and the endpoint's single
  DeviceTopology connector, then walks at most 16 PnP ancestors. DeviceTopology
  activation is metadata access; no `IAudioClient` is activated by enumeration.
- Candidate pairing requires distinct explicit render/capture IDs, active opposite
  flows, the same adapter node, hardware ID `VBAudioVACWDM`, and service
  `VBAudioVACMME` from the separately inspected official basic VB-CABLE INF. The
  Windows backend repeats inventory before opening the selected **render** endpoint.
  Names, default endpoints, unrelated virtual audio products and caller-supplied
  metadata alone cannot select an output. Unknown/multiple topology connectors
  remain unsupported. Matching PnP metadata is not signature verification.
- `prepare` requires exactly supported 48 kHz float32, explicitly chosen mono or
  stereo. No closest-format or system-default fallback. Stereo duplicates the
  same original mono samples; invalid/non-finite/out-of-range PCM is rejected.
- WASAPI shared/event rendering requests 20 ms. Actual capacity must be 20–100 ms;
  only 20 ms of silence is primed. One ticket and at most 960 frames per write,
  no retained PCM queue. Full buffer reports backpressure without overwriting.
- A ticket exposes raw clock position/frequency, 100 ns QPC, padding and the next
  submitted sample-frame index. Position units are converted with checked integer
  arithmetic, not assumed to be frames. Clock freshness, backward motion, consumed
  cursor overrun, late ticket, device errors and invalid sources revoke the output
  lease before cleanup. Native snapshots older than 5 ms are rejected. The existing
  Rust `DeviceClock` still owns drift-rate validation; this library does not replace it.
- Each native backend/session is created, used and destroyed on its owner MTA.
  The owner waits on the borrowed event plus its cancellation event; this library
  starts no background thread. `OutputLease` must belong to this generation and
  revoke both lanes without blocking/reentering the session. That joint owner is
  still to be implemented. `Source::self_microphone` is an internal assertion,
  not proof of a physical input's identity; real source admission remains required.
- The 2 ms admission guard and 20 ms ticket limit are experimental bounds, not a
  Windows scheduling guarantee. Preemption between snapshot and buffer handoff,
  bridge consumption latency, and the receiver clock require actual measurement.
  Do not publish synchronized/meeting-ready status from a successful write.

## Stop and restart boundary

`stop` revokes the lease/ticket first, then attempts both WASAPI Stop and Reset,
retaining both errors and allowing explicit cleanup retry. Reset confirms only
the client queue/clock reset. It does **not** confirm the cable's or meeting app's
queue is drained. Windows therefore reports `external_drain_verified=false` and
refuses reuse of that backend. A new C++ object is not evidence of downstream drain;
the future main owner must retain this barrier across object/process recreation.
Do not expose this adapter as a production restart mechanism before that is solved.
The in-memory backend verifies and clears its own queue for three restart tests;
those results do not qualify a real bridge restart.

## Rust bridge

`TingXianAudioOutput.dll` exports the ABI in `audio_bridge.h`; its 24/24/56/48-byte
format/clock/ticket/status structs are asserted on both sides. `audio.rs` locks
the DLL and parent directories, checks the native host's build pin and loaded
path, and uses system-only dependency search. `NativeAudio` and tickets are
`!Send/!Sync`; native exports additionally enforce their creator thread. The
callback context remains alive through native destruction. The borrowed event
can be waited on with the future joint owner's stop signal, without busy polling.

Reservations carry a per-instance `Rc` identity so a ticket surviving destruction
cannot target a new owner even if its native address/generation is reused. Only
exact `S_OK` accepts writes; `S_FALSE` is backpressure, with no empty ticket or
timeline consumption. Clock warmup uses two advancing observations. The frame
cursor is converted to rational device units without premature integer rounding.
Cleanup errors, client reset and external drain remain separate status fields.
The revocation flag is a notification, not yet a paired video stop mechanism.

`TingXianAudioFixture.dll` is a separately named test target. It instantiates only
an in-memory backend and additionally exports Step/Read controls. Those symbols
are absent from the product DLL and no fixture controls are provided in the
public Rust API. Neither DLL is added to the app's installer in this round.

The sinc renderer bounds finite filter overshoots at the final normalized PCM
boundary and reports `saturated_samples` per window and in timeline metrics.
This prevents valid full-scale input from faulting the strict native sink;
ordinary in-range values are unchanged. No AGC, speech generation or broader
input acceptance is introduced. Non-finite/native out-of-range samples still fail.
Full-scale saturation can distort peaks; the count is exposed for later quality
evaluation, not hidden as pristine audio. See [bridge validation](../../docs/eye-audio-bridge-validation-2026-11-07.md).

## Build and harmless checks

Use an x64 MSVC developer shell (locally verified VS 2019, SDK 10.0.19041.0):

```powershell
cmake -S audio/windows -B <D-drive-build-directory> -A x64
cmake --build <D-drive-build-directory> --config Release
ctest --test-dir <D-drive-build-directory> -C Release --output-on-failure
```

CTest runs 12 in-memory cases, metadata-only enumeration, and rejection of two
hard-coded nonexistent IDs before audio-client activation. The probe only accepts
`--probe` and `--reject-unknown`; it has no streaming CLI. Public output contains
counts, not endpoint names/IDs. No driver/virtual device registration or audio
capture/playback occurs. A valid probe with zero candidate pairs is an inventory
result, never successful virtual-microphone output.

No third-party driver, binary or model is bundled here. VB-CABLE remains a separately
downloaded, uninstalled candidate; naming, commercial redistribution, installation,
native writes, receiving capture, unplug/error behavior and actual downstream drain
need their own evidence/authorization. See the [AV scope](../../docs/eye-av-sync-scope-2026-11-07.md).

## Primary references

- [Connected adapter lookup](https://learn.microsoft.com/en-us/windows/win32/api/devicetopology/nf-devicetopology-iconnector-getdeviceidconnectedto)
  and [PnP property access](https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_get_devnode_propertyw).
- [Exact format support](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported),
  [actual buffer capacity](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getbuffersize),
  and [current padding](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getcurrentpadding).
- [Clock position](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getposition)
  and [Reset](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-reset).
