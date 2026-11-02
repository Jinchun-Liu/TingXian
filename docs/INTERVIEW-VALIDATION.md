# Public Alpha validation scope

The first public source snapshot derives from local commit `a107408c962b479bb4aa3bd094108f03e0d3fa76`.

The source baseline was exercised on Apple Silicon macOS 14.6.1 with headphones and a two-device Feishu test. The original tester confirmed separate remote/self display and context-aware follow-up, microphone off/on, no self-triggered answer calls, stop/start without restarting the app, and clear/start while capturing. These observations are limited to that environment and are not a fresh acceptance test of every public-release change.

The publication pass replaces personal defaults with synthetic examples and generic prompts, adds fork-specific documentation and source CI, disables upstream binary updates, and excludes private local history. It does not include the work-in-progress larger-model comparison or whole-utterance refinement.

Current publication checks are recorded below after execution. No live API call or real recording is part of these checks. Complete 60-minute continuous-call stability, 30 end-to-end audio latency measurements, and cross-platform acceptance remain outstanding.

## Checks executed for publication

- Scheduler regression: passed (source isolation, full remote turn boundary, queue limits and reset).
- Real React control regression: passed (mic gating, stop/restart, clear/restart, Trash failure preservation).
- TypeScript `tsc --noEmit --incremental false`: passed.
- `interview-core`: 7 unit tests passed after replacing a personal-profile fixture with a synthetic one.
- Upstream portable GGML verifier: 14 tests passed.
- Tracked source scan: passed for credential patterns and excluded private artifact types; this is not a security audit.

The public snapshot has not been rebuilt into a complete desktop bundle or tested in a fresh live call during this publication pass. Upstream formatting warnings remain; source checks do not apply a repository-wide reformat.
