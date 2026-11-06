# 听弦开发约定

- 先读 `docs/PRD-v2-progress.md`、`docs/PRD-v2.1-eye-face.md` 和 `docs/PRD-v2.0.md`；冲突按 v2.1 的视觉范围与优先级处理，使用当前代码/运行证据核对状态。
- 旧数字人、照片注册、口型、TTS 与 R11 代理冻结。新视觉优先 R10E 本地眼部；脸部只规划禁用入口与接口，不能自行续作模型或全代理。
- UI 以已发布 Mac 0.6.5（f80ba86）为基准，保留正确的 Windows 系统/快捷键提示和保存保护。
- Preserve unrelated worktree changes. Never force-push shared history.
- Avoid unnecessary secondary labels and descriptions. Let the layout do the explaining.
- 本人发言仅入上下文。对方完整发言按人数模式触发。AI 草稿不能充当本人实际说过的内容。
- 三路并发按问题隔离，取消/清空后旧事件不得回填。展示窗口只读，不采集、不持有 Key、不独立请求模型。
- 默认跟随 AI 正文，手动翻阅暂停，返回最新才恢复。字幕、本人发言、问题纠错不抢位置。
- macOS 删除移入废纸篓，失败停止并询问。Windows 用户资料必须可恢复清理。不要为了构建擅自清空用户资料、缓存或旧版本。
- 不提交密钥、真实简历、数据库、录音或会话。公开测试用虚构资料。私人交接也不含明文凭证。
- 升级/退出前保护采集状态、未保存输入和会话。应用标识与旧资料格式不随品牌重命名。
- 明确区分源码实现、合成测试、CI 安装、实机短测和 60 分钟真实会议。只报告实际完成的范围。
- Windows 构建从 `packaging/windows/README.md` 开始，CPU 包必须无开发工具可用。CUDA 与数字人单独验收。
