# Contributing

This is an experimental independent Meetily fork. Start with README.md for setup and data flow; see NOTICE.md for upstream provenance.

1. Fork this repository and create a focused branch.
2. Use synthetic documents and audio. Never commit credentials, real resumes, recordings, transcripts, generated databases or model files.
3. Run `node scripts/test-interview.cjs` and `node scripts/test-interview-controls.cjs` after installing frontend dependencies. For core changes run `cargo test -p interview-core --locked` with the documented environment.
4. Describe the problem, behavior change, checks run, and remaining limitations in the PR. Audio changes also need a consented real-device check; indicate when this has not been performed.

Keep microphone speech as context only; automatic suggestions are triggered by explicit remote-utterance completion. Preserve original ASR text and distinguish model suggestions from confirmed personal facts. History cleanup must use recoverable macOS Trash and report failure without silently deleting user data.

By contributing, you agree that your contributions are licensed under the repository's MIT License. No external API access is required for the default source checks.
