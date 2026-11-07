# 岗位准备与跨块整合续作清单

2026-11-08，源码候选。本项延续 CV/面经工具工作，不替代会话压缩 C0–C5。

## 实际补充

已有 preparation/consolidation 输入哈希和结果复用机制保持。新增 `preparation-progress`、`consolidation-progress` 两个只读命令，让宿主从当前来源、任务、预算重建计划后，核对中断前保存的响应。无需手工对照两组 hash，也不创建新的后台进程、数据库或模型客户端。

命令逐块区分可复用、缺失、无效和重复；列出过期输入和没有有效身份的响应位置。引用与完成状态使用原收集器同一校验器。所有重复版本，包括内容相同或已过期的版本，都必须显式选择；不会静默选最后一个，也不会自动付费补跑。无效引用不能因 hash 相同而复用。父准备候选或分组失效时，整合阶段先停止，不能跳过前置阶段。

`ready_to_collect` 只表示分块结构检查满足收集前提；总包大小、语义评审、桌面当前来源核对仍须通过。所有报告保留 `quality_pass=false`、`model_usage=null`、`actual_cost_cny=null`；输入字节上界不当作实际 tokens、单次窗口或节省金额。更换模型后的质量对照仍需宿主模型记录，不能由正文推断运行模型。

工具仅读取宿主已保存的 JSON 数组，不保证宿主在取消前保存了全部答案。没有保存的内容无法恢复，工具也不负责后台 worker 的任务锁或账本；原背景 worker 已有的恢复功能没有重复实现。

## 验证

本次目标回归 32 项通过：preparation 13、consolidation 9、四角色/按需规则 harness 10。其中新增 3 项，既有来源更新和真实 CLI 测试增加续作断言；这些都是虚构输入和人工响应夹具。

- 变更一份资料：未变化的三块仍可复用，变化块待补，旧 hash 仅作为过期记录。
- 中断数组、伪引用、未经证实的完成状态、重复版本、无效身份和过大集合均不能标为完整。
- 资料库身份、岗位、预算改变使对应结果过期；跨块依赖仍需重建。
- 实际 Python CLI 运行前后比较所有输入文件字节，确认进度检查不修改输入。
- 四角色同版本审阅、来源约束、按需规则、实时 legacy 与 Harness² 关闭的既有检查继续通过。

复现：

```sh
python3 -B scripts/test-toolkit-harness.py
python3 -B scripts/test-toolkit-preparation.py
python3 -B scripts/test-toolkit-consolidation.py
python3 -B scripts/export-toolkit.py --out /new/path/TingXian-Toolkit.zip
```

导出采用拒绝覆盖模式。命令示例和响应状态见[工具说明](../interview/toolkit/references/preparation.md#中断后续作)。包内文件校验和源码提交由导出 manifest 记录；当前提交 CI 与完整导出包验证另记交付清单，不借用旧提交 CI。

## 边界

没有新增付费调用、凭证读取、真实四角色推理、Codex/Claude Code 模型运行、桌面回导或安装切换。安装版仍为 0.7.0-beta.5；三路回答、AI2 默认关闭和实时按需路径未改。C5 独立测试授权仍待确认，原先 35 文字/12 视觉套件没有启动。原 PRD 的真实语义、自然度、费用、云转写、视觉和长会议验收继续保留。

本次只有小型源码及工具导出，不生成安装包或 payload；更新前可用空间约 12 GiB，无新增可清理的旧安装副本，没有删除资料或缓存。
