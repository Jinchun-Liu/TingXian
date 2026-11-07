# P1：六题真实模型小试

本模块在 P0 的官方核心适配器上接入真实 DeepSeek 调用。仅使用六道已冻结的虚构开发题，不用于正式资料；策略不能自动启用。本轮费用上限经用户明确设为 **30 元**，探测、执行器、优化器和 B 组选择器共同记账。

## 冻结的比较方式

- A：现有 `background.py` 的原始 MAP/REDUCE 提示，执行一次。
- B：相同固定提示独立执行四次，再由固定规则的模型选择一份；不改策略，不建经验库。
- C0：官方 `run_task(k=1, agg)`；一次基线、两个候选、一次组合执行，三个声明式策略编辑会话；无跨任务经验。

所有角色都用 `deepseek-flash`、high thinking、8192 输出上限。A 每题费用上限 1 元，B/C0 每题均为 3.5 元；全局 30 元优先，分组上限不会叠加成消费授权。最多两个 worker，最多 700 次请求，90 分钟的新请求发起上限。无自动重试；同一请求未完成/usage 未知时保留预留，不冒充未收费。连续三组失败则取消整个小试。

B 与 C0 有相同费用上限，但不为凑齐预算而发送无用请求；实际消耗可能不匹配。若两组最终费用差异大于方案容差，不能宣称“同预算更优”。本轮只有一次开发集探索，不是 144 组正式盲测，也不进行显著性判断。

## 请求和权限

可信 broker 持有凭证与中央 SQLite 账本，worker 通过有长度限制的 stdin/stdout JSON 请求模型。模型只能返回 JSON 数据，无工具、任意 URL、搜索、文件执行或代码修改权限。worker 仅允许读取当前任务/上下文和固定脚本，只写自己的运行目录；网络与评价标注均不可访问。优化器同样在这种子进程中运行，不能继承主进程的 API Key。

候选只允许修改受限策略对象，三个固定边界文件仍必须完全匹配。文档中的指令均作为不可信材料。引文、格式与源哈希继续由原有打包器检查；原本的语义规则不能被候选改写为宿主配置。

每次请求先按 UTF-8 字节加固定封装余量计算保守输入上界，并加入输出、安全余量，限制到 262144。收费预留采用 2026-11-07 核对的官方 **高峰、缓存未命中价**：Flash 输入 2 元/百万 tokens、输出 8 元/百万 tokens。缓存与空闲时段优惠不释放预算。返回 usage 后记录同口径估算，不把它叫作实际账单；usage 缺失时保留整笔预留。实际 token 数超出预留上界会停止后续请求。

[官方人民币价格](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/) · [官方请求与 usage 格式](https://api-docs.deepseek.com/api/create-chat-completion/)

## 运行

先按上层 README 准备固定上游与独立 Python 环境。只在独立实验中设置 `TX_H2_API_KEY`，不要写入仓库。`authorization.json` 必须明确 `stage: P1-six-dev-pilot`、`approved_budget_cny: 30` 和代码中六个固定任务 ID。输出目录若已有 `manifest.json`，会拒绝直接重跑；先审计已有调用和费用，再单独设计恢复，不重置账本或暗中重试。

```sh
python -B experiments/harness2/pilot/test_broker.py /private/tmp/tx-h2-budget-tests-01
python -B experiments/harness2/pilot/test_workers.py \
  --upstream /private/tmp/tx-h2-upstream --output /private/tmp/tx-h2-workers-01
python -B experiments/harness2/pilot/run_pilot.py \
  --upstream /private/tmp/tx-h2-upstream \
  --dataset /path/to/p0-output/dataset \
  --output /path/to/independent-pilot \
  --authorization /path/to/authorization.json
```

在 `output/broker/CANCEL` 创建空文件即可请求取消：已发送请求可能仍产生费用，返回后拒收；不再发新请求，检查到取消后正在运行的本地 worker 被终止（在途 HTTP 最多等待其 150 秒超时）。SIGINT 也设置持久化取消。凭证只保留在本次进程内存，输出文件只保存无凭证请求正文、模型最终 JSON、usage 和必要服务元数据。

## 评价限制

所有组级提交冻结后，独立无网络评价进程才能读取标注。当前只计算“原文原子条目是否由连续引文覆盖”和“类别是否精确匹配草案”两个代理指标，并保留逐项证据。它无法发现 note/profile/settings 中的所有幻觉，也不能代替语义判断。

标注仍待人工复核，人物混淆、冲突是否被正确表达、设置建议和人工修订成本需另行审阅。不得把模型输出通过 JSON/引文校验当作真实背景正确；不得据此宣称 Harness² 质量提升或正式可用。

## 本轮之后的可靠性修正

首轮冻结源码 `47c3358` 的 B 协调器遇到单个候选失败会提前退出，官方 C0 则继续处理其余候选。这一差异会混入完成率，首轮结果只作探索记录。修正版 B 固定执行四个槽位，保留失败，只从合法产物中选优；失败不会触发重复生成。

`amend_fixed_review.py` 为这次开发集提供一次明确编号的补充：旧产物只作为不可变的选优输入；已经失败的槽位保留，仅执行当时未尝试的槽位。原 `pilot-report.json`、原费用快照、源码和结果不变，补充放在 `amendment-01/`，所有费用继续累计到同一 30 元账本。它不是新的盲测，也不是对旧失败的重试。

修正版还在发送模型请求前核对 CNY 计费币种；取消和结果提交使用同一把锁；截断/格式错误时保留最终答案通道和 usage 诊断，不保存内部推理正文。实际执行的冻结源码、后续源码修正及验证结果分别记录，不把离线回归当作整轮模型重跑。
