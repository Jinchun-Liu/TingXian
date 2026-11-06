# 安装与运行（工具包 2.0.1）

完整复制 `interview-background` 文件夹，不要只复制 SKILL.md。已有同名目录时先核对版本，不覆盖用户修改。

| 本地宿主 | 项目范围 | 个人范围 | 调用 |
| --- | --- | --- | --- |
| Codex | `<项目>/.agents/skills/interview-background` | `~/.agents/skills/interview-background` | `$interview-background` |
| Claude Code | `<项目>/.claude/skills/interview-background` | `~/.claude/skills/interview-background` | `/interview-background` |

上述目录在 2026-11-06 按 [Codex 官方说明](https://learn.chatgpt.com/docs/build-skills#where-codex-loads-local-skills) 和 [Claude Code 官方说明](https://code.claude.com/docs/en/skills#choose-where-skills-load) 核对。适用于有本机文件访问权限的会话；云端/托管宿主需按其安装入口导入，不能假定能读取本机目录。加载后确认技能名称出现，再开始。路径与权限以实际宿主为准。

需要 Python 3.9+；MD/TXT/JSON/YAML 的文本提取及校验仅用标准库。PDF 与 DOCX 还需固定版本依赖，建议在独立虚拟环境运行 `python -m pip install -r <技能目录>/requirements.txt`。Windows 可用 `py -3` 替代 `python3`。扫描 PDF 不进行隐式 OCR，必须报告待处理。

给 Agent 的示例请求：

> 用 interview-background 整理我指定的资料文件夹，使用高推理能力模型，逐文件、逐批次提取有来源的背景；把中间文件放在另一个工作目录。保留冲突、日期、本人贡献与未完成状态，生成设置建议，最后校验并输出新的 .meetily.json。不要直接修改听弦设置或原文。

在 Agent 开始前说明目标资料目录和结果目录。Agent 的模型服务可能接收其读取的资料；本工具不自带 API Key，也不另发网络请求。`desktop` 是桌面应用的内部入口，不在外部 Agent 工作流调用。

可先运行 `python3 <技能目录>/scripts/self_test.py`。它在新的临时目录用附带虚构资料验证分块、续作、打包、格式校验和来源变更拒绝，不调用模型、不写听弦数据库。通过仅证明工具流程，不代表该 Agent 实际完成了背景推理或桌面导回验收。

真实流程完成后运行 `python3 <技能目录>/scripts/background.py validate --bundle <结果.meetily.json>`，再在听弦「资料与设置 → 资料库 → 导入 Agent 结果」审阅。岗位等设置需另行逐项勾选。保留 cards.json、profile.json、来源文件和所用模型记录以便续作；由用户决定是否清理。
