# v0.3.0-alpha.1 validation scope

The v0.2 baseline froze development source at HEAD `8f4eeeacdff20f801cb18cf6b23916c1883b735a` plus the adaptive-answer and layout edits present at snapshot time. It includes the newer local transcription, profile-review and session features; private source history and real evaluation records are excluded. Subsequent conversation-scene changes are performed only in the independent public checkout, based on public commit `1a257945ae47952c33e1040ec4391cd08d5e345d`.

## Executed locally on Apple Silicon macOS

| Check | Result |
| --- | --- |
| Interview UI regressions | 17 scripts passed, including actual React controls, all four scenes, capture without a Key/cloud consent, pause/resume, explicit learning/meeting actions and labelled AI history |
| Monitoring metrics | 10 Python tests passed in the unchanged v0.2 baseline; CI runs them again for this release |
| Core library | 48 unit/integration tests passed, including local simulated HTTP failures, source conflicts, structured imports and context settings |
| Native Rust library | Final full run: 289 passed, 0 failed, 4 explicitly ignored |
| TypeScript | `tsc --noEmit --incremental false` passed |
| Frontend production export | `next build` passed; retained upstream lint/dependency warnings |
| Synthetic profile demo | Unchanged v0.2 fixture: 3/3 files and 10 offline retrieval queries; no live API validation |
| UI verification | Actual React component tests and production export; the earlier v0.2 browser layout inspection is not a fresh v0.3 desktop acceptance test |
| Public source scan | Passed tracked-file credential-pattern and private-artifact checks; not a security audit |

The native suite uses the downloaded build-time FFmpeg through a temporary test PATH. All 289 runnable tests passed; four explicit opt-in tests remained ignored.

## Scope and limits

No real AI request, microphone capture, meeting call, signed installation package or fresh 60-minute stability run was performed for this release. Hardware/system-audio tests and the explicit real-Trash exit test remain opt-in; their ignored status is not a pass. Configurable languages are input options, not measured multilingual accuracy claims. The scenario prompts have deterministic payload coverage, including old-config compatibility and separation of AI history from actual speech and confirmed facts; their live answer quality is not independently accepted.

Earlier private testing of the baseline covered separate source display, remote-only suggestions, contextual follow-ups, microphone switching and stop/clear/restart. Those observations do not establish acceptance of every change in this public version. Thirty end-to-end audio latency measurements, sustained-call behavior and cross-platform acceptance remain outstanding.

Tests used synthetic data and a dummy process credential. The publication copy and its build output were isolated; the active development application was not restarted or replaced.
