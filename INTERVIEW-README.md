# 面试练习助手开发入口

当前公开版本的安装、功能、数据流与限制见 [README](README.md)，验证范围见 [验证说明](docs/INTERVIEW-VALIDATION.md)。

开发目录：

- `frontend/src/app/interview/page.tsx`：持续对话、资料设置、麦克风及历史控件。
- `frontend/src/lib/interview-scheduler.ts`：对方完整发言的队列与去重。
- `frontend/src-tauri/src/audio/`：双通道采集、语音活动分段、转写及生命周期。
- `frontend/src-tauri/src/interview.rs`：资料索引、会话与模型调用命令。
- `frontend/src-tauri/src/interview_history.rs`：历史归档和 macOS 废纸篓。
- `crates/interview-core`：文档抽取、检索、提示词及 DeepSeek 流。
- `crates/interview-core/src/preferences.rs`：通用场景和语言配置，空白默认值。
- `frontend/src/components/interview/InterviewPreferences.tsx`：岗位、背景、术语和语言设置。
- `examples/profile`：虚构的背景、简历与参考答案。

`interview/questions.json` 是跨行业题目样例；`selection.example.json` 相对于所选资料根目录。运行离线检索示例：

```bash
bash -c 'source scripts/interview-env.sh; cargo run -p interview-core --example validate -- examples/profile "$INTERVIEW_TOOLS/public-validation"'
```

仅显式添加 `--live` 时该示例调用付费 API。实机音频用例须在独立获准的测试通话中验证，自动测试不代替真实通话验收。
