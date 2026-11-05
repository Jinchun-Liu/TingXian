# Windows runtime notices

TingXian is MIT licensed and retains the Meetily notices. Third-party components keep their own licenses.

- Python 3.12.10 embedded x64 is from python.org. Its PSF license is in `python/LICENSE.txt`.
- python-docx, pypdf, lxml, PyYAML and typing_extensions are pinned in `interview/requirements.txt`. Their wheel metadata and license files are retained under `python/Lib/site-packages`.
- Whisper small multilingual weights use the upstream OpenAI Whisper MIT license, included as `licenses/Whisper-LICENSE.txt`. The exact GGML weight hash is in `runtime-lock.json`.
- ONNX Runtime 1.22.0 uses MIT. `onnxruntime-LICENSE.txt` is installed beside its DLLs. Download and file hashes are pinned in `frontend/src-tauri/build/onnxruntime.rs`.
- FFmpeg is the LGPL shared build from [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-11-04-12-51). The archive's licenses, documentation and build configuration are preserved under `licenses/ffmpeg` and `licenses/ffmpeg-build.txt`. Its replaceable DLLs are beside `ffmpeg.exe`. Corresponding FFmpeg source is [a35157abb9](https://github.com/FFmpeg/FFmpeg/tree/a35157abb9), with third-party source/build recipes in the linked BtbN build project. See those licenses for source and redistribution obligations. No GPL or nonfree FFmpeg configuration is accepted by the packaging script.
- Microsoft Edge WebView2 is redistributed by the installer under Microsoft's runtime terms. It is not part of TingXian's MIT license.

The base installer contains a CPU Whisper build. It does not distribute CUDA, a local LLM, voice clones or avatar weights.
