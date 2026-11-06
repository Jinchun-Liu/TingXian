# EyeCorrectionProvider boundary

This is an opt-in native development module. The base app, display window,
prerequisite worker, CPU installer and virtual camera do **not** invoke it yet.
It has no capture, audio, recording preference, UI, cloud or face-replacement API.

`EyeCorrectionProvider` accepts packed BGR8 frames and returns a separate corrected
buffer or an error. `Session` owns explicit preparation and generation admission:

- Off initially; at most one inference and two waiting frames. Overflow discards
  the oldest pending frame. IDs, dimensions, length and monotonic age are checked.
- A 250 ms lease is checked before and after processing. The output sink must
  also enforce its own lease/watchdog, as the existing video source does.
- No-face, SDK failure, malformed results or sink refusal revoke output, clear the
  queue and enter fault. There is no raw fallback and no automatic resume. A user
  must stop/clean up and explicitly begin another generation.
- Stop revokes output immediately without waiting for a vendor call. The worker
  cleans up when that call returns; stale initialization and results cannot emit.
  Cleanup uncertainty permanently blocks this process from restarting.
- Provider calls use one worker execution lane. The session owns no threads.
  The host must join that lane after stop before destruction, and supervise it in
  a killable process for hangs/crashes. The existing worker supervisor is a future
  integration point; it is not connected to this module yet.
- Sink callbacks run under the generation lock and must be short, non-reentrant,
  synchronous and `noexcept`. They acknowledge handoff, not meeting receipt.

## SDK-free checks

```text
cmake -S video/eye -B <new-build-directory> -DCMAKE_BUILD_TYPE=Release
cmake --build <new-build-directory> --config Release
ctest --test-dir <new-build-directory> -C Release --output-on-failure
```

These use synthetic bytes and a fake provider only. CI on Windows and Linux has
no NVIDIA dependency and cannot certify the SDK, GPU, quality or performance.

## Explicit AR 0.8.7 offline experiment

`TINGXIAN_AR087_OFFLINE_LAB` defaults OFF. Enabling it requires Windows x64,
headers from the reviewed [official source commit](https://github.com/NVIDIA-Maxine/Maxine-AR-SDK/tree/f54db7ecaa0534b696d6194ac7252fc39e4d94a1),
and a separately reviewed local `ar087_local_pins.h` build input. It defines
`RuntimeRoot` and `RuntimePins` (relative path, size, SHA-256) for the complete
official protected installation tree. This is a local development pin, **not**
an approved production manifest or runtime redistribution grant.

```text
-DTINGXIAN_AR087_OFFLINE_LAB=ON
-DTINGXIAN_AR087_SOURCE=<fixed-official-source>
-DTINGXIAN_AR087_PINS=<reviewed-local-pin-header-directory>
```

The adapter hashes and holds read-only file locks, rejects reparse/hardlinked
files and unlisted tree entries, and loads absolute DLL paths with restricted
search. It does not use the sample proxy, CWD/PATH discovery, global CUDA
installation or an application-directory override. It checks the runtime ABI
version and synchronizes the GPU before consuming metadata and output. The C
bridge avoids the SDK header's C++ inline constructors making unpinned imports.

The isolated experiment accepts **explicit offline raw files only**:

```text
tingxian-eye-offline --offline-bgr-v1 <width> <height> <frame-count> <input.bgr> <new-output.bgr>
```

Files use packed BGR8. Input length must exactly match the declared frame count;
output uses `CREATE_NEW`. Decode/encode containers separately and check return
codes, frame counts, dimensions and audio policy. On a processing fault, one
neutral frame is written, the tool stops and exits nonzero; a partial file is
diagnostic evidence, not a completed video. Empty/default arguments never open
a camera. Logs contain status/counts, not frames or user paths.

`--offline-bgr-restart3-v1` uses the same arguments and performs three complete
prepare/process/stop lifetimes in one process, appending three independent passes
to the new output file. It resets IDs and temporal state; it is an explicit local
restart check, not a live capture mode.

Before testing licensed material, block this exact executable's outbound network
access and validate the active rule. `local_only` describes processing location;
it is not a sandbox or proof that a vendor binary cannot attempt telemetry.
Current-process path locks do not defend against an administrator/kernel attacker.
Production activation, permissions, signing trust, approved file pins, process
supervision, network policy, model loss/recovery and packaging remain separate.

The adapter uses only GazeRedirection, with no voice/identity generation. The
sample-compatible one-face output is **not** verified user identity or robust
multi-person rejection. Out-of-range naturalness and multiple-person privacy
need dedicated acceptance before camera/output integration.

## Experiment decision and evidence

Hypothesis: the boundary can enforce bounded work, neutral failure and stop
isolation while replacing the provider independently. Decision: whether to use
this module inside the native worker. Stop on raw fallback, stale delivery,
unbounded queue or unsafe load; diagnose before retrying. Affected artifacts:
provider contract, worker integration and Windows acceptance ledger.

Only our own synthetic results belong in public CI. Runtime license 8.9 limits
publication of SDK benchmark/regression/performance results without permission;
keep SDK traces, outputs, metrics and licensed footage local. See
[acquisition and license boundaries](../../docs/eye-ar087-acquisition.md).

The API call sequence follows the official MIT-licensed headers and
[programming guide](https://docs.nvidia.com/deeplearning/maxine/ar-sdk-programming-guide/index.html).
No NVIDIA binary, model, header or sample source is copied into this module.
Runtime/model and media licenses remain independent of our code.
