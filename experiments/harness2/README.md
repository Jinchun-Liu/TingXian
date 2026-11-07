# Harness² × 听弦：隔离接入试验

本目录实现方案的 **P0 无模型接入验证**，不是正式版功能，也不是模型质量评测。使用官方冻结核心调度听弦现有资料打包器；不读应用数据库、不录音、不调用模型、不加载真实背景、不自动安装或启用候选策略。

## 接入位置

```text
Harness² run_task(k=1, agg)
  → TingXianAdapter
  → 只含声明式文本的候选策略
  → macOS 沙箱内固定 worker
  → 原有 background.py 的读取、引文、格式、哈希校验
  → 待人工核对的 bundle.meetily.json
```

路线为 U。冻结上游 `6868817518876de8fd170c6a8354daca273b6191`，无上游补丁。调用 `harness2.core.schedule.run_task`、`BenchmarkAdapter`、`BenchSpec`、`SurfaceSpec`、`Proposer`/`Reply`；本轮每题一次基线、两个候选执行和一次组合执行。官方示例还单独验证了其跨任务经验库调用；听弦适配器没有接入可激活的长期经验库。

上游提示模板只认识既定 surface 名称，因此适配器声明 `SurfaceSpec.name='opencode'`；只是模板兼容，不会安装、启动 OpenCode 或调用其模型接口。独立 namespace 为 `tingxian-text`。若以后升级上游，必须重新核对这个兼容点。

## 复现（macOS）

ZIP 中的 `source/` 是可独立运行本试验的最小源码快照；可以在其中执行下列命令。没有 `.git` 时清单的 `host_commit` 为空，另外记录已冻结的原宿主基线和实际文件哈希。

Python 3.12/3.13；已实测 macOS 14.6.1 arm64 / Python 3.12.0。不要复用正式听弦的虚拟环境。

```sh
git clone https://github.com/google-research/harness2.git /private/tmp/tx-h2-upstream
git -C /private/tmp/tx-h2-upstream checkout --detach 6868817518876de8fd170c6a8354daca273b6191
python3.12 -m venv /private/tmp/tx-h2-venv
/private/tmp/tx-h2-venv/bin/python -m pip install --isolated -r experiments/harness2/requirements-p0.txt

# 在听弦源码根目录执行。输出必须是尚不存在的新目录。
env -i PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 \
  /private/tmp/tx-h2-venv/bin/python experiments/harness2/run_p0.py \
  --upstream /private/tmp/tx-h2-upstream --output /private/tmp/tx-h2-smoke-01
env -i PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 \
  /private/tmp/tx-h2-venv/bin/python experiments/harness2/test_p0.py \
  --upstream /private/tmp/tx-h2-upstream --output /private/tmp/tx-h2-faults-01
```

`sandbox-exec` 是本轮 Mac 隔离边界。若外层开发工具的沙箱禁止创建嵌套沙箱，需在获得宿主许可后运行外层命令；**不允许删除子进程沙箱来绕过失败**。非 Mac 平台直接拒绝执行，Windows/Linux 隔离尚未实现。

官方无 Key 示例可从上游目录运行 `python -m harness2.cli.demo --output <新路径>/demo.json --k 1`。官方 core 没有强制第三方依赖；本目录的固定依赖用于听弦文档解析。

## 产物与边界

| 文件 | 用途 |
| --- | --- |
| `experiment-manifest.json` | 上下游版本、代码/样本哈希、解析依赖、禁用收费执行的状态 |
| `dataset/cases.jsonl`、`inputs/` | 24 份虚构资料，12 开发 / 12 盲测预留；本轮全部仅做机械传输测试 |
| `dataset/evaluator/labels.jsonl` | 独立标注草案，worker 无读取权限；未完成人工复核，不可称作已通过的盲测 |
| `experiment/runs/` | 官方核心归档的基线、候选与组合过程 |
| `experiment/executions/`、`runs.jsonl` | 每次 worker 状态、源/策略签名、校验后的资料包；失败也保留 |
| `smoke-report.json` | 完成数量和来源校验；不含质量提升指标 |
| `strategy-candidate.json` | 由实际选中且验证过的执行导出的声明式策略，始终为 candidate |

`SmokeProposer` 是固定脚本，只修改排序和审阅文本。worker 实际只使用排序参数，将原文连续引文统一放入 `technical` 类，保留“未做语义确认”标记，设置建议为空。这样能检验真实读取、校验和打包链路，**不能证明事实分类、冲突识别、语义完整性或模型能力**。引文长度受现有打包器限制，本轮没有测量事实覆盖率。

候选只能改四个预设文本文件中的声明式 JSON；三个边界文件必须与基线一致。不能写可执行脚本、插件、任意 Skill 或改变宿主校验器。沙箱只读当前任务文件，只有当前运行目录可写；网络、评价标注、其他任务及应用资料不可读写。当前确定性提议器仍运行在可信协调进程；接入真实模型前还需要为优化器/评价器建立独立隔离和网络代理，不能直接赋予模型整个协调进程的权限。保留的是文件内容隔离，不声称隐藏所有文件元数据，也不把它作为任意第三方插件的通用安全沙箱。

取消状态持久化：不再发起新执行，已在运行的本地 worker 最多等到本次完成/超时，其产物不被接纳。相同 run_id 只有策略、源文件、worker 和宿主签名一致且产物哈希仍有效才复用；其他情况保留旧证据并要求新的 run_id。

`control.Ledger` 是待接入收费模型调度的组件：默认禁用；先预留所有在途请求最坏费用；usage 未返回保持未知并占用预留；取消后迟到结果不接纳。每次请求上限为 `min(模型窗口, 262144)`，包含输出及安全余量。当前只有并发/取消/超限故障测试，没有真实提供商计费、tokenizer 或网络重试验证。

## 下一阶段

先补人工标注与评分器、独立模型适配和调用前预算接线，再以经用户确认的模型/费用上限运行 6 题 A/B/C0。完成开发集并冻结策略后，才进入 12 题 × 4 组 × 3 次的真正盲测。正式资料导入、Codex/Claude Code 安装回导、Windows、复杂问答/视觉、真实会议及长期经验激活均需后续验收。方案中的预算建议不视为消费授权。

参考：[官方源码](https://github.com/google-research/harness2/tree/6868817518876de8fd170c6a8354daca273b6191)。本试验不替代上游完整实验，也不宣称复现论文结果。
