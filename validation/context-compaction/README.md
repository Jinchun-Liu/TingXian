# C5 上下文配对试验准备

本目录的 `cases.json` 延续 C4 冻结六题（3 开发、3 保留），没有改题或用于真实模型调参。`context_measure` 仍只是一项确定性预算示例，不能用于真实视觉调用；它包含无效图片占位符。

新增 `context_trial_prepare` 与 `prepare.py` **只准备和核对**，没有网络执行模式、凭证读取或应用数据库访问。`check-authorization` 通过不意味着执行、收费、人工验收或正式启用。独立的 `execute.py` 已接入生产传输和持久账本，须同时满足本批授权、绑定的执行二进制、当天价格、CNY计价及显式 `--execute`；当前只通过合成验证。不能将这里的请求交给无预算脚本批量发送。

## 配对协议

- A：当前短历史；B：按原文检索旧发言；C：只有有效 AI2 候选晋级后才装配记忆。
- 短题使用四条虚构实际发言，A/B/C 请求相同，不建立 AI2 任务；长题四十条，最近十二条入答题，旧二十八条封成 AI2 快照。这是固定协议，不是实际长会议。
- 所有组保持同题、同来源版本、同 DeepSeek Flash、非思考、AI1 4096 输出、262144 应用窗口与 legacy。AI2 独立角色，8192 输出、60秒；不占回答槽位。四角色仍只离线使用。
- 长题 C 默认 `awaiting_real_ai2`，没有伪造“成功压缩”或把人工答案替代模型结果。`materialize` 会调用生产引用/原文/快照校验，跨题、改否定或伪造引用拒绝；成功只说明本地结构校验，不证明输入确实来自模型。执行器还须绑定真实响应及尝试记录。
- C 的准备阶段按完整输入窗口预留费用，不把可变候选猜成一个固定小请求。完成后再次装配、计量并检查预留。AI2 失败则记录失败及 B 回退，不能计成压缩成功。
- 图像是新建的两个公开虚构代码画面，没有采集屏幕，也没有把旧发言或参考答案画进去。原 35 文字/12 图像测试资产未动。两图适用于上下文专项，不替代真实 OCR、屏幕捕获和多模态质量验收。

## 复现

复用既有 Rust/Python/Swift 环境，输出路径必须不存在：

```sh
cargo build --locked -p interview-core --example context_trial_prepare --example context_trial_run
swift validation/answer-vision/render-fixtures.swift validation/context-compaction/visuals.json /new/c5-images
python3 -B validation/context-compaction/prepare.py prepare \
  --binary target/debug/examples/context_trial_prepare \
  --runner target/debug/examples/context_trial_run \
  --images /new/c5-images --out /new/c5-prepared
C5_PREPARER="$PWD/target/debug/examples/context_trial_prepare" \
C5_RUNNER="$PWD/target/debug/examples/context_trial_run" \
  python3 -B -m unittest discover -s validation/context-compaction -p 'test_*.py'
```

Windows 可用同一 Python/Rust 逻辑和已核对哈希的 PNG；Swift 只是 macOS 新图渲染器，不作为跨平台运行依赖。测试用临时资料仅为合成数据并保留，不自动删除。

准备目录含每题输入/原始请求、AI2 快照、请求依赖清单、文件/源码/二进制 SHA-256、价格记录、默认未批准的授权模板、用量/时延空表和人工评分表。默认授权金额为0。真实模型结果、usage、首个有意义答案时延、质量与实际费用都保持空值。每类仅两题属于先导试验，不能宣称可靠的总体 p95 或60分钟稳定性。

```sh
# 只核对独立授权；默认模板应被拒绝。不会发送请求。
python3 -B validation/context-compaction/prepare.py check-authorization \
  --binary target/debug/examples/context_trial_prepare --bundle /new/c5-prepared \
  --authorization /path/to/explicit-c5-authorization.json
# 对另行获得的 AI2 候选进行本地校验和 C 请求装配；不会发送。
python3 -B validation/context-compaction/prepare.py materialize \
  --binary target/debug/examples/context_trial_prepare --bundle /new/c5-prepared \
  --case D02 --candidate /path/to/candidate.json --out /new/D02-C.json
```

授权按本批 manifest 哈希、数据范围、模型/endpoint、当天日期、金额和40次尝试绑定。旧 Harness² 或47题授权均无效；价格需在实际执行当天重新核对，变化时重新准备并获对应授权。公开虚构数据授权不包含私人会话、安装切换或策略晋级。模板更改或 preflight 自身都不是用户授权的证据。

## 预算依据与限制

2026-11-08 核对[官方价格](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/)：Flash 高峰缓存未命中输入每百万2元、输出每百万8元。[视觉说明](https://api-docs.deepseek.com/guides/vision/)给出单图最多1024 token。准备器独立应用该已核对图片预留；生产测量里的 `image_reserve=null` 仍保留，不能冒称本地精确 tokenizer 或真实接口窗口已验收。供应商较大窗口不会提高应用256K基线。

18次AI1 + 4次AI2 = 22次基础调用；AI1每次最多2次传输、AI2不自动重试，最坏40次。金额按整数微元计算：完整序列化文本字节估算 + 图片预留 + 安全余量，以及每次输出上限；长题C按完整配置输入预算预留。当前固定图文准备结果约6.348028元，这是保守计划预留，不是已产生费用或无条件费用保证。未核对的后续价格、超出此协议的调用和人为重跑不在该数值内。

执行器使用 SQLite FULL 事务，在网络子进程启动前将最大尝试预留提交到磁盘；未知usage不得记0或释放预留；失败、取消和重试均计入分母与总额；分别记录AI1/AI2、缓存、实际用量和总开销。不得挪用旧30元额度或假定获得过 Key 就已允许本批消费。总开销收益、口语自然度和保真由实际结果及人工核对决定，不能由输入长度推断。


## 有授权后的执行入口

源码路径：`validation/context-compaction/execute.py` 和 `crates/interview-core/examples/context_trial_run.rs`。下面不含凭证；实际Key仅由获准执行的进程环境 `TINGXIAN_C5_API_KEY` 提供，不读聊天、应用库或现有账号配置。默认命令只作预检，不读Key、不联网。

```sh
python3 -B validation/context-compaction/execute.py \
  --binary target/debug/examples/context_trial_prepare \
  --runner target/debug/examples/context_trial_run \
  --bundle /new/c5-prepared --authorization /path/to/explicit-c5-authorization.json
# 仅在这批明确获准后添加 --execute；中断后继续尚未尝试的任务须再加 --resume。
```

执行前核对账号计价币种为CNY，只留币种，不保存余额。固定顺序每题A、B、必要的AI2、C；最多22任务、40传输尝试。并发文件锁防双启动，SQLite账本记录封存任务、完整预留、结果、原始usage及未知费用。恢复核对账本用量与金额；不重发已保留/中断/已结算任务。未知终态保留全额并停止本轮，显式恢复只能做剩余任务；AI2结果丢失或引用校验失败时C使用B回退并单列标记。

输出上限、进程截止、取消、连续三次失败或超过预留均有停止边界。每次调度前重新核对来源和授权文件；撤回授权停止后续调用。已有请求发生的成本不能撤销，仍保留已知usage或未知预留。跨日价格/原授权过期会阻止恢复，不会自己重开整批。

每段流式事件带时间戳，真正有意义答案的首字时间仍需人工从事件中标注；不将`嗯`、首个非空字符或开始标记冒充该指标。实际模型费用、口语质量、CPU/RSS与长会话仍未测；账本按高峰价计算的usage金额只是保守估值，不是账单。完整故障测试边界见[执行检查点](../../docs/context-c5-execution.md)。
