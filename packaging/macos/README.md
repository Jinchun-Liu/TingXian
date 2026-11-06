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

## PRD v2 Mac 预览配置

`frontend/src-tauri/tauri.macos-preview.conf.json` 将 Mac 测试包标为0.7.0-beta.2，独立于 Windows 配置和原0.6.5默认配置。此版包含设备枚举/停止等待和 Core Audio 启动隔离修复；真实收音仍须单独验收。依赖按根说明安装后，在 frontend 运行：

```bash
./node_modules/.bin/tauri build --debug --bundles app --config src-tauri/tauri.macos-preview.conf.json
```

这条命令沿用既有本地 debug 包基线，嵌入静态前端资源，不依赖开发服务器；不是优化 release 性能验收。CMake 与 Rust 工具需在 PATH 中。配置使用 `npm run build` 构建已有依赖，不自动重新安装共享 node_modules。之后用上述打包脚本，传入实际生成的 debug/bundle/macos/听弦.app。安装前保留当前应用和用户输入；安装自检、恢复草稿、真实音频和60分钟会议分别验收。

签名暂存目录位于系统临时目录并保留供核对；最终 pkg 写到指定输出目录。脚本仅清理生成副本中妨碍签名的 FinderInfo/ResourceFork，不修改源应用，也不清除其他扩展属性。若要节约解包空间，可用 `pkgutil --expand` 保留压缩 Payload，并流式对账普通文件、符号链接和 AppleDouble 元数据；这不代替安装后的实际运行验收。

暂存应用、模型和运行时优先使用 macOS `clonefile` 写时复制：副本拥有独立 inode，签名或修改不会改变输入。非 APFS 或跨卷失败时回退普通复制，不使用硬链接。仍需为两个压缩 pkg、签名改写及后续安装保留空间。
