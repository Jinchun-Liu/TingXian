# Meetily Interview · 面试练习助手

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE.md)
[![Source checks](https://github.com/Jinchun-Liu/meetily-interview/actions/workflows/source-checks.yml/badge.svg)](https://github.com/Jinchun-Liu/meetily-interview/actions/workflows/source-checks.yml)

面向中文技术面试练习的桌面助手：本地转写双方语音，在对方完整发言结束后，结合用户选择的资料生成回答建议。本人发言只作为上下文，不单独触发 AI。

An experimental macOS interview practice assistant with local Whisper transcription, separate system/microphone channels, profile retrieval, and context-aware suggestions via DeepSeek.

**这是基于 [Meetily Community](https://github.com/Zackriya-Solutions/meetily) v0.4.1 的独立修改版，不是官方产品。当前为源码 Alpha，无签名或公证安装包。** 本仓库沿用 MIT 许可证，保留上游版权；见 [来源与变更说明](NOTICE.md)。

## 功能

- 持续采集系统声音和麦克风，按来源显示在长对话列表中。
- 约 1.5 秒静默作为发言结束判据；仅对方的完整发言进入串行回答队列。
- 独立控制“记录我的声音”，停止后可重新开始；麦克风开关不跟随飞书静音。
- 本地 Whisper 中文转写；AI 根据原始文本、最近对话和资料提供保守纠错及建议。
- 导入 MD/TXT/DOCX/PDF，使用 SQLite FTS5/BM25 检索；区分个人事实、原始证据和练习答案。
- 支持冻结建议、修正本人回答、查看证据、导出会话，以及将历史移入 macOS 废纸篓后重新开始。

## 数据流

语音在本机完成 Whisper 转写。**回答功能需要联网，会向 DeepSeek 发送问题、近期对话、事实卡和允许外发的相关资料片段**，并产生 API 费用。音频不会由面试回答模块上传。其他上游页面有各自的模型与网络设置。

仓库不包含密钥、真实简历、个人资料库、录音或真实会话。`examples/profile` 全部是虚构示例。API Key 可在应用设置中临时输入，或通过 `DEEPSEEK_API_KEY` 环境变量提供。请勿提交自己的资料和凭据。

仅在参与者知情、场景允许时使用转写和辅助功能；本工具用于练习及获准的辅助场景。

## 从源码运行

已验证的开发平台：Apple Silicon、macOS 14.6.1、16 GB 内存。系统音频路径要求 macOS 14.4+；Windows/Linux 的本分支功能尚未验收。

先准备 Node.js/npm、Python 3 和 Xcode Command Line Tools（`xcode-select --install`）。设置脚本会在仓库相邻的 `.meetily-toolchain` 中安装隔离的 Rust 1.99.1、pnpm 9.15.9 及 Python 依赖，不改写全局 shell 配置。首次构建需要网络和较多磁盘空间，并下载构建依赖与 FFmpeg；`--with-model` 下载校验后的 Whisper base 多语言模型。

```bash
git clone https://github.com/Jinchun-Liu/meetily-interview.git
cd meetily-interview
bash scripts/setup-interview.sh --with-model
bash scripts/build-interview.sh
```

然后双击 `启动面试辅助.command`。构建产物位于 `target/debug/bundle/macos/Meetily Interview Local.app`。这是调试构建，未公证。

1. 在“资料与设置”选择自己的资料目录，扫描、选择资料类型及片段外发许可后导入。可先选择仓库的 `examples/profile`，预览三份示例资料。
2. 在事实卡中填写真实背景与目标岗位，按需修改术语别名。软件不预设你的学校、公司或项目经历。
3. 配置自己的 DeepSeek Key，下载并选择 Whisper **base 多语言**模型；按 macOS 提示授予所需音频权限。
4. 使用耳机，勾选两项许可后点击“开始持续聆听与回答”。本人讲话时可开启本软件的麦克风，AI 不会因此新增回答请求。
5. 结束前导出需要保留的会话；退出应用后的完整历史恢复尚未实现。

如需要环境变量方式，参考 [密钥示例](interview/deepseek.env.example)。不要把实际值写入该示例文件。

## 验证

```bash
# frontend dependencies are installed by setup-interview.sh
node scripts/test-interview.cjs
node scripts/test-interview-controls.cjs
bash -c 'source scripts/interview-env.sh; cargo test -p interview-core --locked'
```

GitHub Actions 运行调度器和真实 React 控件回归、TypeScript 检查及发布文件检查；不代表完成桌面打包或真实通话验收。首个公开版本基于已验证的本地功能快照，新增通用化改动的检查范围见 [验证说明](docs/INTERVIEW-VALIDATION.md)。

## 当前限制

- 中文近音词、中英混合及专有名词仍可能识别错误；AI 纠错也可能误解。核对姓名、数字、否定词和个人经历。
- 静默判据可能在长停顿时提前结束一轮；长语音上限、排队及模型推理会影响延迟。
- 声音来源是采集通道，不能证明真实说话人身份。扬声器可能造成麦克风重复收音，系统音频可能包含其他应用声音。
- 60 分钟连续通话稳定性与 30 次端到端延迟验收尚未完成；本版本未包含开发中的 small 模型整段复核。
- 禁用面试音频自动保存不等于无落盘：转写、元数据、索引和浏览器恢复缓存仍可能保留。清理历史不清理个人资料索引、手动导出或废纸篓副本。
- 本分支的上游遥测已禁用。自动更新已关闭，避免本分支被上游安装包覆盖。更新请拉取源码并重新构建。

## 贡献与许可

欢迎提交 Issue / Pull Request，见 [贡献指南](CONTRIBUTING.md)。请提供脱敏的复现步骤，不上传真实音频、简历或 Key。安全问题见 [SECURITY.md](SECURITY.md)。

代码使用 [MIT License](LICENSE.md)。第三方依赖、模型、字体、FFmpeg 等遵循各自许可证，模型和本机构建工具不随源码分发。上游架构与其他构建资料保留在 `docs/`，其功能、安装包及发布流程描述不等同本分支能力。
