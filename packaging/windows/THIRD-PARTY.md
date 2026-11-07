# Windows runtime notices

TingXian is MIT licensed and retains the Meetily notices. Third-party components keep their own licenses.

- Python 3.12.10 embedded x64 is from python.org. Its PSF license is in `python/LICENSE.txt`.
- python-docx, pypdf, lxml, PyYAML and typing_extensions are pinned in `interview/requirements.txt`. Their wheel metadata and license files are retained under `python/Lib/site-packages`.
- Whisper small multilingual weights use the upstream OpenAI Whisper MIT license, included as `licenses/Whisper-LICENSE.txt`. The exact GGML weight hash is in `runtime-lock.json`.
- ONNX Runtime 1.22.0 uses MIT. `onnxruntime-LICENSE.txt` is installed beside its DLLs. Download and file hashes are pinned in `frontend/src-tauri/build/onnxruntime.rs`.
- FFmpeg is the LGPL shared build from [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-11-04-12-51). The archive's licenses, documentation and build configuration are preserved under `licenses/ffmpeg` and `licenses/ffmpeg-build.txt`. Its replaceable DLLs are beside `ffmpeg.exe`. Corresponding FFmpeg source is [a35157abb9](https://github.com/FFmpeg/FFmpeg/tree/a35157abb9), with third-party source/build recipes in the linked BtbN build project. See those licenses for source and redistribution obligations. No GPL or nonfree FFmpeg configuration is accepted by the packaging script.
- Microsoft Visual C++ x64 CRT/OpenMP redistributable DLLs are deployed app-locally from the build machine's Visual Studio Redist directory. The exact files are in the runtime manifest, with the source version in `licenses/msvc-runtime.txt`. They are covered by [Microsoft runtime terms](https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/), not MIT.
- Microsoft Edge WebView2 is redistributed by the installer under Microsoft's runtime terms. It is not part of TingXian's MIT license.

The base installer contains a CPU Whisper build. It does not distribute CUDA, a local LLM, voice clones or avatar weights.

## Rubato 0.15.0

The native original-voice timeline uses the public sinc interpolator from
[Rubato 0.15.0](https://docs.rs/rubato/0.15.0/rubato/), already pinned in Cargo.lock.
The archive SHA-256 is `b5d18b486e7d29a408ef3f825bc1327d8f87af091c987ca2f5b734625940e234`.
Its MIT notice is retained below; this does not grant rights to NVIDIA models or
third-party virtual audio drivers. No such driver is bundled by this change.

Copyright (c) 2020 Henrik Enquist

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
