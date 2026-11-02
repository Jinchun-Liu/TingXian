# Origin and modifications

This is an independent derivative of Meetily Community v0.4.1 by Zackriya Solutions, licensed under the MIT License. It is not an official Meetily release and is not endorsed by the upstream maintainers.

- Upstream: https://github.com/Zackriya-Solutions/meetily
- Upstream baseline: `a2cb62e827da7ef59f65064c97233efb2313878e`
- Local interview feature snapshot: `a107408c962b479bb4aa3bd094108f03e0d3fa76`
- Public source release: `v0.1.0-alpha.1` (upstream app compatibility version remains 0.4.1).

Modifications add continuous bidirectional interview dialogue, local document retrieval, contextual text correction and suggestions through DeepSeek, remote-only answer scheduling, independent microphone control, and recoverable history cleanup. Public-release changes replace personal defaults with synthetic samples, document data transmission, and disable the upstream binary update feed and telemetry initialization.

This repository begins with a sanitized source snapshot. Local development history, personal validation records, credential files, recordings, generated binaries, and unfinished model comparison changes are not distributed. Upstream release workflows are replaced with source checks; legacy workflow documentation is retained for reference only.

The upstream MIT copyright notice is retained in LICENSE.md. Vendored cidre retains its original license and its local adaptation is documented in vendor/cidre/LOCAL-PATCH.md. Dependencies and optional model/FFmpeg downloads retain their own licenses. The legacy backend/whisper.cpp submodule is pinned to `d682e150908e10caa4c15883c633d7902d385237` and is not needed for the main Tauri interview workflow.
