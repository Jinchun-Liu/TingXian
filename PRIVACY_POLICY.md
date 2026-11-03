# Data handling in this fork

This document describes the conversation workflow, not a guarantee about every retained upstream page.

- System audio and optional microphone audio are captured locally and transcribed with Whisper. The microphone switch is independent of Feishu mute.
- The conversation answer module sends text to DeepSeek: the current utterance, bounded recent dialogue, the user-supplied facts card, the configured conversation context and terminology, and retrieved excerpts marked for external transmission. API usage is billed by the provider. It does not send raw audio.
- Profile extraction and SQLite search happen locally. An external-transmission flag controls retrieved excerpts; it does not remove words already spoken or entered into the facts card.
- A Key entered in the conversation settings is held in the process state. An optional environment file is outside the source tree. Other upstream provider settings may use different storage.
- The conversation entry disables automatic audio-file saving, but transcripts, metadata, indexes and browser recovery data may remain on disk. Explicitly saved interview sessions can be restored locally. Unsaved AI answers are not guaranteed to survive exit.
- Clear history stops capture and requests, then archives recognized interview history into macOS Trash. Failure leaves history available. Profile indexes, settings, models, manual exports and Trash copies are retained. This is not secure erasure.
- Source releases contain synthetic examples only. Disable unrelated system sounds and obtain appropriate participant permission before capturing conversations.

The upstream analytics initializer is disabled in this fork and contains no upstream telemetry project key.

The public application uses `io.github.jinchunliu.meetilyinterview` and the separate `.meetily-interview-tools` tool directory by default. It does not automatically migrate a private development installation. Scene settings are saved in the local profile database, and are sent as context when requesting an answer. Profile indexes, saved sessions, recovery residue and exports have different retention scopes; inspect the selected cleanup action before use.

Local transcription does not require an API key or text transmission permission. General, learning and meeting scenes start in transcription-only mode; interview mode enables automatic remote-utterance requests only with a configured key and transmission consent. General/learning/meeting requests may include up to three bounded previous AI suggestions, explicitly labelled as generated content. Those suggestions are not actual participant speech, confirmed facts or meeting decisions. Explicit recent-discussion summaries use bounded context, not necessarily the full recording.
