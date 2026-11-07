# 听弦教育扩展：开源 Skills 与 Harness 调研

核对日期：2026-11-07。用途：支持 PRD v2.2 的选型与后续实施。结论基于官方仓库、固定提交中的文件及官方工具文档；本轮没有安装、启动或执行第三方项目，也没有验证其在听弦中的教学效果。

## 1 接入结论

首轮保留听弦现有任务调度与数据层，选择性改编 OpenMAIC 的教学策略，结合 ai-teaching-skills 的单步提示、讲解和对照反馈设计。Promptfoo 列为开发期评测工具候选。先交付会话复盘和单题重答，再接教练练习与模拟面试。

这次核对没有发现一个已经证明能直接嵌入听弦、支持 Windows、覆盖所有面试类别且无需适配的成熟教育 Skill 包。教育平台和提示策略的成熟度不同：OpenMAIC、DeepTutor 有持续发布和较完整的系统；小型教学 Skill 更易改编，但不能凭可下载、Release 或 Stars 宣称教学有效。通用 LangGraph、Promptfoo 和 ts-fsrs 的工程积累较长，分别解决编排、评测和复习时间问题，不提供现成的面试教学内容。

| 项目 | 主要能力 | 首轮决定 | 后续触发条件 |
| --- | --- | --- | --- |
| OpenMAIC | 职业训练、费曼解释、课程目标、分层帮助、课堂工作流 | 审查并改编少量声明式教学策略，保留许可与来源 | 教练/模拟阶段用固定案例验证 |
| ai-teaching-skills | 苏格拉底提问、用户讲解、对照讲评 | 参考模式隔离与反馈设计；不照搬全局拒答 | 在现有反馈模板确有缺口时选用 |
| Promptfoo | 模型与提示回归、断言和多提供方评测 | 开发依赖候选；先写与工具无关的案例 | 现有测试难以表达开放式质量对比时接入 |
| DeepTutor | 来源关联学习、问答、测验、学习记录与技能扩展 | 参考数据契约；整套运行时后置 | 确有课程、资料阅读等需求时另做适配验证 |
| CodeTrain Open Skill | 编程任务、用户提交、提示与检查 | P2 专项候选 | 安全执行环境和 Windows 宿主兼容有证据 |
| LearnHarness | 计划、学习、练习及可检查的学习记录 | 早期产品参考，首轮不复制代码或 Skill | 单独完成许可、数据迁移和平台评估 |
| LangGraph | 检查点、暂停恢复、人工介入、有状态执行 | 不预先引入 | 已证明当前调度无法满足恢复需求 |
| ts-fsrs | 间隔重复调度 | P2，可选概念卡片调度 | 有真实复练记录、工具链兼容且用户需要自动排期 |

## 2 固定版本与维护快照

以下是本次获取源码文本使用的默认分支提交。最后列是 API 查询当时返回的最新列示 Release，**不表示该 Release 与固定提交相同**。创建和推送时间是 GitHub 元数据，不能代替代码审查或使用验证。

| 仓库与官方入口 | 建仓 / 最近推送（UTC 日期） | 本次固定提交 | 最新列示 Release |
| --- | --- | --- | --- |
| [OpenMAIC](https://github.com/THU-MAIC/OpenMAIC) | 2026-03-11 / 2026-11-06 | 5bd8cc840ee6397d063e1fe6aba974b9f0401593 | [v1.4.1](https://github.com/THU-MAIC/OpenMAIC/releases/tag/v1.4.1) |
| [DeepTutor](https://github.com/HKUDS/DeepTutor) | 2025-12-28 / 2026-11-06 | 60fb3bdfddb30b7740c7ce3bd9b28d908f2af6db | [v1.7.2](https://github.com/HKUDS/DeepTutor/releases/tag/v1.7.2) |
| [ai-teaching-skills](https://github.com/bstellato/ai-teaching-skills) | 2026-09-02 / 2026-09-04 | 6d3f85726e7b0fedacc8a078e174161293777044 | 未列出 |
| [CodeTrain Open Skill](https://github.com/InferHaven/agent-skills) | 2026-06-23 / 2026-10-31 | 8eba05dc8550b2320dac137bceec6dae8572d0b4 | [v1.1.0](https://github.com/InferHaven/agent-skills/releases/tag/v1.1.0) |
| [LearnHarness](https://github.com/amosblomqvist/learnharness) | 2026-10-16 / 2026-11-04 | 713377f366771e3e4730e6acd1bc290e0ebc6d6e | [v0.5.0](https://github.com/amosblomqvist/learnharness/releases/tag/v0.5.0) |
| [LangGraph](https://github.com/langchain-ai/langgraph) | 2023-08-09 / 2026-11-06 | e5e37b5d70dbfd93c7006bd90a658c6179a95946 | [sdk==0.4.14](https://github.com/langchain-ai/langgraph/releases/tag/sdk%3D%3D0.4.14)，SDK 子包版本 |
| [Promptfoo](https://github.com/promptfoo/promptfoo) | 2023-04-28 / 2026-11-07 | db6a5682c17c1a834bc729813119a4ee14b76041 | [0.125.1](https://github.com/promptfoo/promptfoo/releases/tag/0.125.1) |
| [ts-fsrs](https://github.com/open-spaced-repetition/ts-fsrs) | 2023-03-05 / 2026-11-06 | edc4ceb79f08dcd41917c35c7eb14c8b09526100 | [v6.0.0](https://github.com/open-spaced-repetition/ts-fsrs/releases/tag/v6.0.0) |

八个仓库在核对时均未归档。精确时间、Release URL 和 GitHub 识别的许可证保存在 research/upstreams.json；实际许可证文本另行获取，不能只依赖 API 标签。

## 3 教学 Skills：可复用内容和需要剥离的依赖

### 3.1 OpenMAIC：优先选择教学行为

上游包含课堂、测验、项目式学习、持久化会话与内置教学 Skill；仓库树可见测试、评测、端到端测试和 CI 文件。这里只确认文件存在，未执行测试或核实每个流水线是否通过。完整平台要求 Node ≥22.19、pnpm ≥10、PostgreSQL 16 等，直接内嵌会显著增加当前桌面应用的依赖。[固定版 README](https://github.com/THU-MAIC/OpenMAIC/blob/5bd8cc840ee6397d063e1fe6aba974b9f0401593/README.md)

已获取并阅读的五个 Skill：

| Skill | 可借鉴的具体行为 | 听弦适配与验收 |
| --- | --- | --- |
| [vocational](https://github.com/THU-MAIC/OpenMAIC/blob/5bd8cc840ee6397d063e1fe6aba974b9f0401593/skills/agent-runtime/vocational/SKILL.md) | 由职业任务组织操作顺序、工具、成功条件及错误后果 | 将课堂组件改成面试题/评分锚点；检查是否能说明验证步骤与交接 |
| [feynman-learning](https://github.com/THU-MAIC/OpenMAIC/blob/5bd8cc840ee6397d063e1fe6aba974b9f0401593/skills/agent-runtime/feynman-learning/SKILL.md) | 用户先解释，找到最小概念缺口，再解释与迁移 | 去除自动课堂和 TTS 调用；保存用户解释与受助记录 |
| [zone-of-proximal-development](https://github.com/THU-MAIC/OpenMAIC/blob/5bd8cc840ee6397d063e1fe6aba974b9f0401593/skills/agent-runtime/zone-of-proximal-development/SKILL.md) | 独立尝试、定向帮助、撤去帮助、可比新题、证据记录 | 该 Skill 元数据为 0.2.0-experimental；不输出稳定能力或“ZPD 分数”，单独评估 |
| [understanding-by-design](https://github.com/THU-MAIC/OpenMAIC/blob/5bd8cc840ee6397d063e1fe6aba974b9f0401593/skills/agent-runtime/understanding-by-design/SKILL.md) | 先确定目标和评价证据，再安排练习 | 用于内容包编写；不要求一次生成完整课程或专有页面 |
| [curriculum-planner](https://github.com/THU-MAIC/OpenMAIC/blob/5bd8cc840ee6397d063e1fe6aba974b9f0401593/skills/agent-runtime/curriculum-planner/SKILL.md) | 前后知识衔接、分阶段进度和持久化 | 只采纳目标与进度结构，去除上游宿主工具及固定多轮确认流程 |

根许可证为 MIT，但 README 明确列出其他组件许可，包括 packages/mathml2omml 的 LGPL-3.0-or-later。选择性改编必须记录具体文件来源和变更，不能把整仓依赖默认视为 MIT。[许可证](https://github.com/THU-MAIC/OpenMAIC/blob/5bd8cc840ee6397d063e1fe6aba974b9f0401593/LICENSE)

### 3.2 ai-teaching-skills：小而清晰，但仍属早期

该仓库包含三份教学策略：苏格拉底式辅导、用户向初学者解释、基于初答和参考的对照讲评。适合审阅、删减和建立反例测试；建仓较新、没有列示 Release，不应称为成熟教学平台。[README](https://github.com/bstellato/ai-teaching-skills/blob/6d3f85726e7b0fedacc8a078e174161293777044/README.md)

苏格拉底策略对直接解答有严格限制。听弦只在教练模式按需采用单步提示；用户可以查看示例并留下“已受助”标记，不能把上游拒答规则应用到实时辅助。[苏格拉底 Skill](https://github.com/bstellato/ai-teaching-skills/blob/6d3f85726e7b0fedacc8a078e174161293777044/ai-socratic-tutor/SKILL.md)

对照讲评强调先看学习者的实际尝试，定位第一个实质差异，接受不同有效解法；这与 M1 的证据反馈相容。初答不存在时不能补造。[对照 Skill](https://github.com/bstellato/ai-teaching-skills/blob/6d3f85726e7b0fedacc8a078e174161293777044/ai-solution-study/SKILL.md)、[MIT 许可证](https://github.com/bstellato/ai-teaching-skills/blob/6d3f85726e7b0fedacc8a078e174161293777044/LICENSE)

### 3.3 CodeTrain Open Skill：编程专项候选

开放 Skill 提供本地练习界面，强调学习者自己编写代码，再获得提示、检查和复盘。它不是其商业网站和商业 CLI 的开源授权。上游 README 的 2026-10-06 宿主记录报告了多个宿主结果，其中较早 Codex CLI 小模型组合存在提示依从和提交评阅问题；这不是当前 Codex、Windows 或听弦已通过的证据。[README 与宿主测试记录](https://github.com/InferHaven/agent-skills/blob/8eba05dc8550b2320dac137bceec6dae8572d0b4/README.md)

本轮获取了 codetrain/SKILL.md、Apache-2.0 LICENSE 和 NOTICE，未执行安装脚本。后续需要适配任务/提交契约、限定工具权限、设计代码执行沙箱并验证 Windows。首版代码题仅讨论与静态检查，不以引入该 Skill 为理由执行用户代码。[Skill](https://github.com/InferHaven/agent-skills/blob/8eba05dc8550b2320dac137bceec6dae8572d0b4/codetrain/SKILL.md)、[LICENSE](https://github.com/InferHaven/agent-skills/blob/8eba05dc8550b2320dac137bceec6dae8572d0b4/LICENSE)、[NOTICE](https://github.com/InferHaven/agent-skills/blob/8eba05dc8550b2320dac137bceec6dae8572d0b4/NOTICE)

## 4 教育 Harness：参考完整流程，避免首轮系统迁移

### 4.1 DeepTutor

DeepTutor 将资料、问答、测验、研究、解题、阅读和学习记录组织为可扩展工作流，提供 CLI 和结构化输出，并支持 Skills。它适合参考来源关联、学习记录和模块边界；完整运行时的部署、模型配置、依赖和桌面嵌入仍需单独验证。[README](https://github.com/HKUDS/DeepTutor/blob/60fb3bdfddb30b7740c7ce3bd9b28d908f2af6db/README.md)

本次获取的根 SKILL.md 是使用 DeepTutor CLI 的工作说明，不能把它当成脱离运行时的“一份万能教育提示词”。EduHub 可用于发现更多教学内容，但未逐包检查其许可证与行为，不批量安装。[SKILL.md](https://github.com/HKUDS/DeepTutor/blob/60fb3bdfddb30b7740c7ce3bd9b28d908f2af6db/SKILL.md)

根为 Apache-2.0，另有第三方通知。初期参考自有数据契约；如果后来复制代码、模板或工具，需要逐项保留对应声明。[LICENSE](https://github.com/HKUDS/DeepTutor/blob/60fb3bdfddb30b7740c7ce3bd9b28d908f2af6db/LICENSE)、[第三方通知](https://github.com/HKUDS/DeepTutor/blob/60fb3bdfddb30b7740c7ce3bd9b28d908f2af6db/THIRD_PARTY_NOTICES.md)

### 4.2 LearnHarness

该项目围绕测量、计划、教学和练习共享学习记录，强调可检查的证据和本地数据。理念与教育扩展相关，但仓库 2026-10-16 才创建，本次看到 v0.5.0，不能因名称叫 Harness 就当作经过长期验证的基础设施。[README](https://github.com/amosblomqvist/learnharness/blob/713377f366771e3e4730e6acd1bc290e0ebc6d6e/README.md)

其 skills/learnharness/SKILL.md 主要指导 Agent 读取已有 LearnHarness 数据，不等于独立教学引擎。根许可证为 AGPL-3.0；AGPL 不禁止商业使用，但直接复制或组合发布需要遵守适用义务，不能简单改标 MIT。首轮仅用于产品比较，不向听弦源码导入其受许可内容。[Skill](https://github.com/amosblomqvist/learnharness/blob/713377f366771e3e4730e6acd1bc290e0ebc6d6e/skills/learnharness/SKILL.md)、[LICENSE](https://github.com/amosblomqvist/learnharness/blob/713377f366771e3e4730e6acd1bc290e0ebc6d6e/LICENSE)

## 5 通用工具：限定使用范围

### 5.1 LangGraph

LangGraph 提供有状态执行、持久化、暂停恢复和人工介入等能力，采用 MIT。它不包含听弦所需的教学目标、面试题和评分内容。这里核对的是 Python 仓库；JavaScript 方案属于另一个仓库，未在本轮获取或验证。[固定版 README](https://github.com/langchain-ai/langgraph/blob/e5e37b5d70dbfd93c7006bd90a658c6179a95946/README.md)、[LICENSE](https://github.com/langchain-ai/langgraph/blob/e5e37b5d70dbfd93c7006bd90a658c6179a95946/LICENSE)

先用现有调度完成一条“取消—重启—恢复—拒绝迟到事件”链。如果满足需求，停止框架比较。只有明确缺少必要语义时才引入；OSS 库、托管平台和商业可观测服务分别核对，不能混称一个免费依赖。元数据中的 sdk==0.4.14 不是 LangGraph 核心版本。

### 5.2 Promptfoo

Promptfoo 是 MIT 评测工具，适合固定合成数据、多提供方输出比较、结构断言和 CI 回归。它不是教育评分标尺，项目需自行维护来源隔离、帮助记录、引用正确性、无答案泄漏和合理替代解法等案例。[README](https://github.com/promptfoo/promptfoo/blob/db6a5682c17c1a834bc729813119a4ee14b76041/README.md)、[LICENSE](https://github.com/promptfoo/promptfoo/blob/db6a5682c17c1a834bc729813119a4ee14b76041/LICENSE)

后续接入时设 PROMPTFOO_DISABLE_TELEMETRY=1、PROMPTFOO_DISABLE_UPDATE=1，不启用分享上传。默认使用合成数据与本地假提供方；真实模型和评审模型仍会向对应服务发送输入，应单独按用户授权执行。不能把“评测器在本地”描述为“模型数据绝不外发”。[官方遥测文档](https://www.promptfoo.dev/docs/configuration/telemetry/)

### 5.3 ts-fsrs

ts-fsrs 是 MIT 间隔重复调度库，用于决定概念卡片下一次复习时间，不能测量面试综合能力。本次 v6 的使用文档要求 Node ≥24；需核对听弦工具链和实际浏览器打包兼容，不能未经评估升级全项目运行环境。[包文档](https://github.com/open-spaced-repetition/ts-fsrs/blob/edc4ceb79f08dcd41917c35c7eb14c8b09526100/packages/fsrs/README.md)、[LICENSE](https://github.com/open-spaced-repetition/ts-fsrs/blob/edc4ceb79f08dcd41917c35c7eb14c8b09526100/LICENSE)

库版本、FSRS 算法版本和另行提供的优化器包是不同概念。M1 只需保存复练动作；可编辑日期优先，自动调度留待 M3。方案权衡和项目表达继续使用变式任务，不换成背卡次数。

## 6 听弦的复用契约

采用 [Agent Skills 规范](https://agentskills.io/specification)中的目录结构和 SKILL.md 基础字段，另设 manifest.json 表达听弦自己的约束。格式兼容不意味着任何宿主都能支持同一工具；规范中的 allowed-tools 仍有宿主差异。

拟纳入 manifest 的字段：

- id、version、适用模式、教学目标和依赖；
- 上游仓库、固定 commit、文件路径、许可证、版权与变更说明；
- 输入与输出 schema、证据引用格式、预算和允许能力；
- 合成样例、拒绝或降级条件、质量审查状态。

首版只允许声明式策略。上游 SKILL 文本中的 shell、MCP、课堂写入、TTS、安装或下载指令不赋予执行权限；本轮也未执行。导入的策略不能更改系统规则、读取无关资料、调用展示窗口模型或改变用户事实来源。

策略按模式加载：assist 沿用现有回答行为，coach 允许请求提示/示例，mock 延后反馈且记录求助。平台级许可和单包许可分别核验，未知来源的目录包保持待审。

## 7 已获取材料及证据边界

本地获取了八个仓库的 README 和根许可证，以及选定的 Skill/通知/包说明。共 **30 份参考文件，其中 11 份 SKILL.md**；另外保存上游元数据、目录树和分仓清单。没有克隆完整代码仓库、下载模型权重、注册账户或安装依赖。

11 份 SKILL.md 的构成为：OpenMAIC 5、ai-teaching-skills 3、CodeTrain 1、DeepTutor 1、LearnHarness 1。后两项包含运行时使用/数据读取说明，不应统称为 11 个可直接运行的教学能力。

每份参考文件在 education-research/source-manifest.json 中记录固定 URL、相对路径、字节数和 SHA-256。源码目录树、API 元数据和辅助下载脚本不计入上述 30 份参考文件。

获取时 GitHub 连接器的部分内容请求失败；Python HTTPS 尝试也遇到 SSL EOF。随后使用 PowerShell 的正常 HTTPS 请求成功获取固定提交文本，没有关闭 TLS 校验或安全防护。失败记录保存在 research/acquisition-python-failed.json，成功内容以最终清单及哈希校验为准。

本次完成：

- 官方来源检索、版本与许可文本获取；
- 选定 Skill 内容审阅及宿主依赖辨识；
- 听弦接入决定和 PRD 约束；
- 文件完整性与交付文档检查。

本次未完成：第三方项目安装运行、完整传递依赖审计、Windows 适配、听弦代码集成、真实模型评测、真人学习试点。源码中存在测试与 CI 文件不等于本轮通过这些检查。

## 8 后续验证的决策价值

| 验证 | 假设 | 能改变的决定 | 停止条件 |
| --- | --- | --- | --- |
| 证据反馈合成链 | 自有数据契约足以完成一次复盘与重答 | M1 是否可进入真实模型短测 | 来源污染、引用伪造或保存失败时先修根因 |
| 教学策略小样例 | 改编策略能优于现有单次反馈，且不增加无效追问 | 默认启用哪些策略 | 没有可复核改善或出现误导则停用该策略，不扩大模型矩阵 |
| 恢复链验证 | 现有调度足够支撑教育任务 | 是否需要 LangGraph | 满足取消/恢复/隔离即停止框架比较 |
| 内容包试点 | 独立题目与评分包能扩展第二类别 | 何时开放新类别 | 缺来源或专家审阅时只保留草稿，不宣称全类别成熟 |

真实模型、真人试点和外发资料按主进程届时有效授权执行。详细任务、验收 ID 和里程碑见 PRD-v2.2-interview-learning.md。
