# Local build patch

Original: https://github.com/yury/cidre at `a9587fa`, cidre 0.11.3, MIT (LICENSE.txt).

Only build.rs differs: when `CIDRE_USE_CLT=1` on macOS, compile the selected original pomace Objective-C sources with `xcrun clang` and archive with `ar`. Other platforms and builds without this opt-in retain upstream xcodebuild.

Meetily's dependency selects the frameworks actually used by its audio code instead of cidre's `full` default (which sets a macOS 15 target). This local build targets macOS 14.4 and was compiled with the installed macOS 14.4 SDK. No replacement CoreAudio/ASR engine was introduced. Builds and real capture still require separate validation.
