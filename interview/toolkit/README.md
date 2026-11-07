# 听弦 CV / 面试工具包 0.1

候选实现，默认不改变桌面答题策略。四角色用于离线准备与评审，实时只选择适用方法。Harness²关闭，无模型传输或凭证；不支持以审计通过自动启用候选。

## 入口

背景处理继续使用 `../skills/interview-background/scripts/background.py` 的 plan/read/pack 和桌面“导入 Agent 结果”；不是新数据库。`harness.py archive --bundle result.meetily.json --library-id <用户自定库ID> --role <岗位>` 生成完整引用档案和待筛选岗位视图；它是派生核对报告，不能代替桌面事务导入。人工修订通过 `--revisions` 区分 user_asserted，原文引号与hash不变；`--excluded` 接收当前排除来源ID数组。当前岗位视图尚未模型排序，完成状态为unknown需复核。

`harness.py policy --context context.json --preview` 仅装配该任务方法，context含tool(background/realtime)、mode、question_types、role(R01/R07/R08/general)、seniority、language和question。完整注册表离线保存；返回文本、ID、来源映射、hash和估算。未传preview只选择validated规则，目前无已激活条目。

`harness.py team-audit --run run.json` 校验独立评审结果，完整字段见 [执行契约](references/execution.md)。无模型fixture执行器只测编排；真实宿主完成各角色后提交结构化结果。初评相互不看结论，最多三版，每版四角色，最后候选必须有三份同hash评审。未知用量为null；attempts_complete=false时底层调用总数也未知。此工具不保证语义质量。

源码中的 `cargo run -p interview-core --example toolkit_prepare` 从stdin读取question/facts/dialogue/preferences/method_text，只准备与生产流程相同的上下文和精简候选，不支持 --run 或Key。保留旧策略、事实边界和流式实现。

## 项目内安装

导出包有 skills/（四个角色和原背景执行组件）、toolkit/。在你选择的项目中，将两目录原样放到 `.agents/`（Codex）或 `.claude/`（Claude Code）：角色入口将位于 `.agents/skills/tingxian-career-lead/SKILL.md` 或 `.claude/skills/tingxian-career-lead/SKILL.md`，共享资源位于同级toolkit。不要只拷贝四个SKILL.md，也不要覆盖已有同名技能或全局配置。

安装位置已按 [Codex官方说明](https://learn.chatgpt.com/docs/customization/overview) 和 [Claude Code官方说明](https://code.claude.com/docs/en/skills) 核对。仅目录和链接校验，不代表两宿主安装、推理或桌面回导已经实测。Python 3.10+；背景PDF/DOCX依赖见原执行组件requirements.txt。未自动安装任何技能。

## 验证与边界

102规则条目=24 CV+52面经+14口语+12产品约束，78来源记录包括10口语来源。全部保留来源限制，视频元信息不准入正文结论；32方向仅索引，首批后端/功能测试/测开。来源核验依据为交接包，未重新联网核对每个来源。

validation/development.json六个开发案例，spoken.json十二个口语输入；独立保留集在交付核对目录，不放进优化器包。固定原35文字+12图片回归未修改。结构测试、候选输入体量、真实模型语义质量、真人朗读、跨宿主回导和实机会议分别报告。

本阶段尚未完成：每条人工修订的桌面编辑/存储、岗位视图语义排序、实时正式策略晋级与路由缓存、真实四角色模型执行、保留集模型评测及真实语义/自然度/费用验收。使用现有“事实更正”维护本人确认卡；archive侧报告不能假称已写回应用。
