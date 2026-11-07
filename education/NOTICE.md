# 教育内容来源与修改

`content-packs.json` 的题目和行为锚点由听弦编写，MIT，状态 draft；不是专业或真人学习验收。
`crates/interview-core/src/education.rs` 的证据反馈提示选择性改编以下教学行为：

| 来源 | 固定 commit / 原路径 | 改编内容 |
| --- | --- | --- |
| THU-MAIC/OpenMAIC | 5bd8cc840ee6397d063e1fe6aba974b9f0401593 / skills/agent-runtime/vocational/SKILL.md | 职业任务的步骤、验证与完成证据 |
| THU-MAIC/OpenMAIC | 同上 / skills/agent-runtime/understanding-by-design/SKILL.md | 先确定目标与证据，再给练习动作 |
| bstellato/ai-teaching-skills | 6d3f85726e7b0fedacc8a078e174161293777044 / ai-solution-study/SKILL.md | 先看初答、认可合理替代路线、只突出一个优先缺口 |

版权及完整 MIT 声明见 `licenses/OpenMAIC-MIT.txt` 与 `licenses/ai-teaching-skills-MIT.txt`。
原文摘要、固定 URL 见 `../docs/education-research/source-manifest.json`。
移除上游课堂工具、自动发声、宿主指令、全局拒绝完整解答和强制反问流程。
没有复制平台代码、第三方组件或安装外部 Skill。最近发展区 experimental 策略本轮未启用。
其余研究材料不随安装包发布。源清单中的 sources 路径是交接资料的相对路径，
原文留在源码外；该目录不是运行时依赖。
