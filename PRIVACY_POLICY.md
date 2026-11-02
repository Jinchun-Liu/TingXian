# Data handling in this fork

This document describes the interview workflow, not a guarantee about every retained upstream page.

- System audio and optional microphone audio are captured locally and transcribed with Whisper. The microphone switch is independent of Feishu mute.
- The interview answer module sends text to DeepSeek: the current utterance, bounded recent dialogue, the user-supplied facts card, and retrieved excerpts marked for external transmission. API usage is billed by the provider. It does not send raw audio.
- Profile extraction and SQLite search happen locally. An external-transmission flag controls retrieved excerpts; it does not remove words already spoken or entered into the facts card.
- A Key entered in the interview settings is held in the process state. An optional environment file is outside the source tree. Other upstream provider settings may use different storage.
- The interview entry disables automatic audio-file saving, but transcripts, metadata, indexes and browser recovery data may remain on disk. Full history restoration after application exit is not implemented.
- Clear history stops capture and requests, then archives recognized interview history into macOS Trash. Failure leaves history available. Profile indexes, settings, models, manual exports and Trash copies are retained. This is not secure erasure.
- Source releases contain synthetic examples only. Disable unrelated system sounds and obtain appropriate participant permission before capturing conversations.

The upstream analytics initializer is disabled in this fork and contains no upstream telemetry project key.
