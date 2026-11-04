# 构建听弦的独立安装包

在 Apple Silicon Mac 上按根 README 准备源码环境。用 `pnpm exec tauri build --bundles app` 在 frontend 中创建 release 应用。安装包使用同一份网页资源与原生二进制，无需前端开发服务器。

准备以下输入：

- 可重定位的 CPython 3.12.13 macOS arm64 独立运行时（本版使用 uv 管理的 python-build-standalone）。用该解释器将 `interview/requirements.txt` 的二进制 wheel 安装到其 lib/python3.12/site-packages。打包脚本会检验重定位后的 PDF、DOCX 与 YAML 依赖。
- [Whisper small 多语言 GGML 权重](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-small.bin)。打包脚本固定 SHA-256。
- [FFmpeg 8.0.1 源码](https://ffmpeg.org/releases/ffmpeg-8.0.1.tar.xz)，在解包目录中运行 `./configure --disable-autodetect --disable-doc --disable-debug --disable-ffplay --disable-ffprobe --disable-network --enable-audiotoolbox --enable-videotoolbox`，然后运行 `make -j4 ffmpeg`。安装包携带此独立二进制、对应源码、编译参数与 LGPL 文本。

从项目根目录运行以下命令，将参数换成实际路径。

```bash
python3 scripts/package-tingxian-macos.py \
  --app target/release/bundle/macos/听弦.app \
  --python /path/to/standalone-cpython \
  --model /path/to/ggml-small.bin \
  --media /path/to/ffmpeg-8.0.1/ffmpeg \
  --media-source /path/to/ffmpeg-8.0.1.tar.xz \
  --out /path/to/new-release
```

输出为安装到 /Applications 的 .pkg 和 SHA256SUMS.txt。安装包按 arm64、macOS 14.4 或更新版本筛选目标。应用默认 ad hoc 签名；拥有相应证书的构建者可通过 TINGXIAN_APP_SIGN 和 TINGXIAN_PKG_SIGN 指定身份。Apple 公证需另行办理。

复验时用 `pkgutil --expand-full` 展开最终 .pkg，在展开得到的 听弦.app 上运行 `codesign --verify --deep --strict` 和 `Contents/MacOS/meetily --installation-check`。自检会实际加载随包 Whisper small、生成并读取内存中的 DOCX/PDF，运行音频工具，不需要账号或 API Key。
