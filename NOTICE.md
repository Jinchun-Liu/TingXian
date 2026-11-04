# Origin and modifications

This is an independent derivative of Meetily Community v0.4.1 by Zackriya Solutions, licensed under the MIT License. It is not an official Meetily release and is not endorsed by the upstream maintainers.

- Upstream: https://github.com/Zackriya-Solutions/meetily
- Upstream baseline: `a2cb62e827da7ef59f65064c97233efb2313878e`
- First public baseline: `a107408c962b479bb4aa3bd094108f03e0d3fa76`
- Current development source HEAD: `8f4eeeacdff20f801cb18cf6b23916c1883b735a` plus locally present adaptive-answer and layout changes, frozen for this publication before generalization.
- Public source release: `v0.4.0-alpha.1`; application/package versions now follow the fork release.

Modifications add continuous bidirectional interview dialogue, local document retrieval, contextual text correction and suggestions through DeepSeek, remote-only answer scheduling, independent microphone control, and recoverable history cleanup. Public-release changes replace personal defaults with synthetic samples, document data transmission, and disable the upstream binary update feed and telemetry initialization.

This repository begins with a sanitized source snapshot. Local development history, personal validation records, credential files, recordings, generated binaries, and private model-comparison results are not distributed. Upstream release workflows are replaced with source checks; legacy workflow documentation is retained for reference only.

The upstream MIT copyright notice is retained in LICENSE.md. Vendored cidre retains its original license and its local adaptation is documented in vendor/cidre/LOCAL-PATCH.md. Dependencies and optional model/FFmpeg downloads retain their own licenses. The legacy backend/whisper.cpp submodule is pinned to `d682e150908e10caa4c15883c633d7902d385237` and is not needed for the main Tauri interview workflow.

The second public release includes whole-utterance refinement, bounded transcription, session restoration, confirmed document metadata, versioned fact cards, import cancellation, request controls and adaptive answers from the development snapshot. It adds neutral configurable scene/language settings, cross-industry synthetic samples, an isolated public app identifier, and broader regression coverage. Source provenance does not imply a new live-call acceptance result.

Version 0.3 names the public application Meetily Dialogue and adds general, interview, learning and meeting scenarios. The repository address, bundle identifier, native command names and data schema names retain their earlier interview identifiers for compatibility. New installs default to general conversation; persisted pre-scenario settings retain interview behavior. Prior AI explanations are bounded and labelled separately from actual speech and confirmed facts. Local capture no longer depends on cloud consent or a Key.

听弦 0.5.0 独立安装包使用 FFmpeg 8.0.1 的本地 LGPL 配置，并附对应源码及构建配置。内置 Whisper small 的许可位于应用资源中的 Licenses。Python 及文档依赖的许可随运行时分发。
