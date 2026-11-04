# 听弦安装包：第三方来源与许可

- Meetily Community：MIT；原版权声明在同目录 Meetily-MIT.md，源码基线与修改见 NOTICE.md。
- cidre：见 cidre-LICENSE.txt。Core Audio 采集通过其 macOS 绑定实现。
- Whisper 多语言 small：见 Whisper-LICENSE.txt；权重 SHA-256 由打包脚本核验。
- FFmpeg：此包使用未启用 GPL 或 nonfree 的 8.0.1 构建。LGPL 文本、对应源代码归档和实际配置分别见 FFmpeg-LGPL-2.1.txt、ffmpeg-8.0.1.tar.xz、ffmpeg-build.txt。
- Python 3.12、python-docx、pypdf、lxml、PyYAML、typing_extensions 等 Python 组件：原许可随解释器和各自的 dist-info 分发，实际文件见 python-licenses.txt（路径相对 Contents/Resources/python）。
- Tauri、React、Whisper.cpp 等构建依赖版本见源码仓库的 Cargo.lock 和 frontend/pnpm-lock.yaml。完整项目与来源：https://github.com/Jinchun-Liu/TingXian 。
