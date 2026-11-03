# 通用对话助手开发入口（兼容旧文件名）

当前公开版本的安装、功能、数据流与限制见 [README](README.md)，验证范围见 [验证说明](docs/INTERVIEW-VALIDATION.md)。

开发目录：

- `frontend/src/app/interview/page.tsx`：持续对话、资料设置、麦克风及历史控件。
- `frontend/src/lib/interview-scheduler.ts`：对方完整发言的队列与去重。
- `frontend/src-tauri/src/audio/`：双通道采集、语音活动分段、转写及生命周期。
- `frontend/src-tauri/src/interview.rs`：资料索引、会话与模型调用命令。
- `frontend/src-tauri/src/interview_history.rs`：历史归档和 macOS 废纸篓。
- `crates/interview-core`：文档抽取、检索、提示词及 DeepSeek 流。
- `crates/interview-core/src/preferences.rs`：四种对话场景、语言配置和旧版配置兼容。
- `frontend/src/components/interview/InterviewPreferences.tsx`：场景、角色、背景、术语和语言设置。
- `examples/profile`：虚构的背景、简历与参考答案。

`interview/questions.json` 是跨行业题目样例；`selection.example.json` 相对于所选资料根目录。运行离线检索示例：

```bash
bash -c 'source scripts/interview-env.sh; cargo run -p interview-core --example validate -- examples/profile "$INTERVIEW_TOOLS/public-validation"'
```

仅显式添加 `--live` 时该示例调用付费 API。实机音频用例须在独立获准的测试通话中验证，自动测试不代替真实通话验收。

`interview_*` IPC 名称、`/interview` 路由和原有数据库名为兼容接口，不能据此推断应用仅支持面试。场景行为见 `frontend/src/lib/conversation-scenarios.ts` 与 `crates/interview-core/src/preferences.rs`。AI 历史建议仅在非面试模式作为有明确来源标记的上下文，不得当成发言或事实。
