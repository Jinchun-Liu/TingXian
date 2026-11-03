# Meetily Dialogue · 通用对话助手

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE.md)
[![Source checks](https://github.com/Jinchun-Liu/meetily-interview/actions/workflows/source-checks.yml/badge.svg)](https://github.com/Jinchun-Liu/meetily-interview/actions/workflows/source-checks.yml)

面向面试练习、学习问答、会议讨论和日常交流的桌面对话助手。本地转写双方语音，按所选场景结合背景资料提供回答、解释或最近讨论摘要。本人发言作为上下文，始终不会自动触发 AI。

An experimental conversation assistant for interviews, learning, meetings and general discussion, with local Whisper transcription, document retrieval and optional DeepSeek assistance. The interface is currently in Chinese; response and transcription languages are configurable.

**基于 [Meetily Community](https://github.com/Zackriya-Solutions/meetily) v0.4.1 的独立修改版，非官方产品。当前为 v0.3.0-alpha.1 源码预览版，无签名或公证安装包。** 保留上游 MIT 版权；见 [来源与变更](NOTICE.md)。

## 场景与默认行为

| 场景 | 开始采集后的默认行为 | AI 用途 |
| --- | --- | --- |
| 通用对话 | 本地转写，手动提问 | 讨论、解释、澄清 |
| 面试辅助 | 配置 Key 并允许外发后，对方完整发言自动进入回答队列；否则仅转写 | 基于真实背景的口头回答建议 |
| 学习问答 | 本地转写，手动提问 | 概念、步骤、例子和误区；支持“解释最近内容” |
| 会议讨论 | 本地转写，手动提问 | 分析问题；支持“整理最近讨论”，区分结论、待确认事项与行动项 |

所有场景都可直接输入文字提问，无需先采集音频。任何场景的本地转写均不需要 Key 或外发许可；AI 请求需单独配置 Key 并允许外发。用户可显式启用对方发言自动回答；本人麦克风不自动触发。点击本人发言的“用这句提问”只复制到输入框，仍需手动发送。

学习、会议及通用场景会提供最多三轮已完成的 AI 历史建议以理解追问，明确标为生成内容，不视为个人事实、实际发言或会议决议。面试场景保持原有边界，不把 AI 历史答案作为本人回答传入。

## 功能

- 选择场景，自定义角色或岗位、组织、对话背景和领域术语，默认无预填个人资料。
- 回答语言可选中文、英文或跟随发言；转写可自动识别或指定中文、英文、日文、韩文、法文、德文、西班牙文。语言选项不代表各语言均已实机验收。
- 持续采集系统声音和可选麦克风，在长对话列表中保留多轮历史。本人发言不触发回答。
- 完整发言边界、有限队列、来源隔离、采集健康提示；独立麦克风开关，停止后可重新开始。
- 本地 Whisper 转写；选择 `small` 时启用整段复核。AI 根据原文和资料保守理解近音词，保留原始转写。领域术语供 AI 理解，不会作为固定词表强行注入听写。
- 按问题复杂度提供简答或逐点展开；可配置模型 ID、等待提示和请求截止时间，显式测试 DeepSeek 连接。
- 导入 MD/TXT/DOCX/PDF/JSON/YAML，用 SQLite FTS5/BM25 检索；结构化文件可选择字段，导入可取消。
- 资料确认状态和有效期、版本化事实卡、来源冲突提示；练习答案与真实个人事实分开处理。
- 保存、恢复和导出会话；查看证据、修正本人回答、保持阅读位置。历史、残留与资料索引提供各自的清理入口。

## 数据流

音频在本机由 Whisper 转写。**回答会向 DeepSeek 发送当前发言、有限的近期对话、事实卡、对话场景设置和允许外发的相关资料片段**，并产生 API 费用。对话回答模块不上传原始音频。其他保留的上游页面有独立的模型与网络设置。

仓库不包含密钥、真实简历、资料数据库、录音或真实会话。`examples/profile` 全部为虚构示例，不应确认为你的个人事实。API Key 可临时输入，或由 `DEEPSEEK_API_KEY` 环境变量提供。见 [数据处理说明](PRIVACY_POLICY.md)。

用于练习及参与者知情、规则允许的辅助场景。

## 从源码运行

开发验证平台为 Apple Silicon macOS。系统音频路径要求 macOS 14.4+；本分支的 Windows/Linux 功能尚未验收。

准备 Node.js/npm、Python 3、Xcode Command Line Tools（`xcode-select --install`）。脚本在仓库相邻的 `.meetily-interview-tools` 中安装 Rust 1.99.1、pnpm 9.15.9 和 Python 依赖；可用 `INTERVIEW_TOOLS` 指定其他工具目录。首次构建需要网络和磁盘空间，会下载构建依赖及 FFmpeg。`--with-model` 下载并校验 Whisper base 多语言模型。

```bash
git clone https://github.com/Jinchun-Liu/meetily-interview.git
cd meetily-interview
bash scripts/setup-interview.sh --with-model
bash scripts/build-interview.sh
```

双击 `启动对话助手.command`（旧启动文件继续兼容）。调试构建位于 `target/debug/bundle/macos/Meetily Dialogue.app`，未公证。

1. 打开“资料与设置 → 对话场景”，选择通用、面试、学习或会议，填写可选的角色、组织、背景、术语和语言并保存。
2. 选择自己的资料目录，扫描并明确选择资料类型和外发许可后导入。试用示例时选择仓库的 `examples/profile`，点击“预览三份示例”。
3. 核对资料状态，在事实卡中明确确认自己的经历和限制。术语别名每行一组，用逗号分隔；不同业务含义不应合并为同义词。
4. 在音频设置准备 Whisper `base` 或 `small` 多语言模型。只用本地转写无需 Key；使用 AI 时再配置自己的 DeepSeek Key、允许外发，并按需测试连接。英文专用 `.en` 模型仅适用于指定英文转写。
5. 音频场景建议使用耳机，授予权限并确认允许转写后开始。麦克风开关与会议软件静音独立；暂停/恢复保留此前的自动回答选择，不会将仅转写切换为自动请求。
6. 结束后可显式保存或导出会话。已保存会话可恢复查看，但不会自动重新采集或请求回答；未保存的 AI 建议不保证退出后恢复。

环境变量配置示例见 [deepseek.env.example](interview/deepseek.env.example)，不要在仓库内保存真实值。

## 升级说明

应用显示名称改为 **Meetily Dialogue**，保留 `io.github.jinchunliu.meetilyinterview` 应用标识、`/interview` 页面路由和现有 GitHub 地址，确保 v0.2 的数据与链接继续可用。

- 新安装默认“通用对话”；v0.2 已保存但未包含场景字段的设置，按“面试辅助”读取，其他内容保留。可在设置中显式切换并保存。
- 场景切换不会清空历史，也不会自动开始采集或请求 AI。恢复旧会话不会自动开启采集/外发许可；旧记录不自动标成新场景。
- v0.1 或私有开发版使用其他应用标识，仍不会自动迁移其资料和 Key，旧目录保留。请在公开版按需重新导入。

使用示例见 [四种对话场景](examples/conversation-scenarios.md)。

## 验证

```bash
node scripts/test-public-source.cjs
python3 scripts/test-interview-monitor.py
bash -c 'source scripts/interview-env.sh; cargo test -p interview-core --locked'
```

测试使用合成资料；网络回归只启动本机模拟 HTTP 服务，不需要真实 Key。GitHub Actions 运行前端回归、TypeScript 和发布文件检查。具体执行结果与未完成项见 [验证说明](docs/INTERVIEW-VALIDATION.md)。

## 当前限制

- 专有名词、近音词和混合语言仍可能识别错误；AI 纠正也可能误解。各场景提示词的回答质量尚需独立评估。
- 静默和长度边界可能拆分长发言；模型推理、整段复核和排队影响延迟。`small` 占用更多资源，不保证优于每种场景的 `base`。
- 系统音频不是可靠的说话人身份；其他应用声音也会被采集。扬声器回声可能进入麦克风。
- “整理最近讨论”仅处理有限近期上下文（最多 12 段转写、合计约 2500 字符，加最近 3 轮问答）；不保证完整会议纪要，不自动分配任务或发送消息。
- 60 分钟连续通话稳定性、30 次端到端音频延迟和跨平台验收尚未完整完成。
- 禁用自动音频保存不等于无落盘：转写、元数据、缓存和索引仍可能保留。清理会移入 macOS 废纸篓；手动导出、模型和废纸篓副本不会自动消失。
- 上游遥测和二进制自动更新已禁用。更新请拉取本仓库源码并重新构建。

## 贡献与许可

欢迎提交脱敏的 Issue / Pull Request，见 [贡献指南](CONTRIBUTING.md) 和 [安全反馈](SECURITY.md)。代码使用 [MIT License](LICENSE.md)；第三方依赖、模型和 FFmpeg 遵循各自许可证。保留的上游文档不代表本分支已实现或验证其全部能力。
