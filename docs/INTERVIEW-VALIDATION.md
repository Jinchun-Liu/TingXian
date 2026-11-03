# v0.2.0-alpha.1 validation scope

This publication freezes development source at HEAD `8f4eeeacdff20f801cb18cf6b23916c1883b735a` plus the adaptive-answer and layout edits present at snapshot time. It includes the newer local transcription, profile-review and session features; private source history and real evaluation records are excluded. Generalization is performed only in the independent public checkout.

## Executed locally on Apple Silicon macOS

| Check | Result |
| --- | --- |
| Interview UI regressions | 17 scripts passed, including actual React controls and configurable scene settings |
| Monitoring metrics | 10 Python tests passed |
| Core library | 46 unit/integration tests passed, including local simulated HTTP failures, source conflicts, structured imports and context settings |
| Native Rust library | Final full run: 289 passed, 0 failed, 4 explicitly ignored |
| TypeScript | `tsc --noEmit --incremental false` passed |
| Frontend production export | `next build` passed; retained upstream lint/dependency warnings |
| Synthetic profile demo | 3/3 files imported, 10 offline retrieval queries completed, no API calls |
| Browser preview | Generic heading and scene controls rendered; browser preview has no Tauri native bridge and is not a desktop functional test |
| Public source scan | Passed tracked-file credential-pattern and private-artifact checks; not a security audit |

The native suite initially passed 288 tests with four explicitly ignored and one failure because FFmpeg was absent from the isolated test executable search path. The checkpoint test passed after adding the downloaded build-time FFmpeg to a temporary test PATH. The corrected full run passed all 289 runnable tests; four explicit opt-in tests remained ignored.

## Scope and limits

No real AI request, microphone capture, meeting call, signed installation package or fresh 60-minute stability run was performed for this release. Hardware/system-audio tests and the explicit real-Trash exit test remain opt-in; their ignored status is not a pass. Configurable languages are input options, not measured multilingual accuracy claims. The new neutral prompt has deterministic payload coverage but no cross-industry answer-quality acceptance.

Earlier private testing of the baseline covered separate source display, remote-only suggestions, contextual follow-ups, microphone switching and stop/clear/restart. Those observations do not establish acceptance of every change in this public version. Thirty end-to-end audio latency measurements, sustained-call behavior and cross-platform acceptance remain outstanding.

Tests used synthetic data and a dummy process credential. The publication copy and its build output were isolated; the active development application was not restarted or replaced.
