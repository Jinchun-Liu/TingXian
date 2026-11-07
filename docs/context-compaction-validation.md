# 会话记忆验证矩阵

2026-11-08。本页记录无真实模型的源码候选证据。V 项为分层覆盖，不表示真实语义或长会议验收全部通过。137 项核心检查包含一个既有 example 用例；61 项原生检查另有 1 项真实废纸篓测试忽略；前端 33 份脚本；原工具 10+11+8 项；类型与生产构建通过。新增图形界面仅用隔离生产构建与 synthetic bridge，没有触碰真实会话。

| 项 | 可复现证据 | 尚不证明 |
|---|---|---|
| V01 | compaction 默认配置 + memory IPC/default-off + session-memory 脚本，无原文写入/AI2 | 安装版开关行为 |
| V02 | request_budget 边界、4096 小窗口及 context_pressure 窗口切换 | 真实 256K 窗口支持 |
| V03 | 完整 JSON/tool/schema/输出 reserve、图片 unknown、usage 细分 | 供应商 tokenizer 精度 |
| V04 | 原生 append → tick → loopback → 持久化晋级；实际 stream 发送前预算观测；最少实际发言用例 | 真正 ASR 40 分钟连续触发；不会伪造整场原文进 200K |
| V05 | 原生三路各自固定 v0，3/1 完成后 AI2 晋级 v1，2 晚到仍 v0；41–48 原文并发保留；后续请求 v1 | 真实网络流式计时 |
| V06 | 整段 ASR 替换、重复终稿不重开、人工修订、原文 revision/hash/CAS | 对任意中文语义改写的保真证明 |
| V07 | 清空 epoch、恢复独立分支、来源版本变化、过期依赖、模型配置失效、重启 Interrupted | 真实权限/进程崩溃中的在途 AI 正文 |
| V08 | 伪造/缺失/跨域引用、否定变更、条件遗漏、过长、异常终态均不晋级 | 自由摘要的事实正确性 |
| V09 | loopback 429/500/取消/异常 finish_reason、发送前取消 0 次、传输后 usage unknown 保留预留 | 服务商账单与真实 timeout 行为 |
| V10 | hard guard 保护当前题/事实/冲突/最近原文/图片；context_length 一次可选缩减重试、错误 usage 保存 | 图片总预算精确可知 |
| V11 | 单飞/60秒冷却/同快照去重/新实际发言批次/每场限额；压力75%/60%回落；相同主机前台取消AI2 | 不同域名共享账号的真实限额识别 |
| V12 | origin/kind 验证、实际发言/请求输入/AI建议分开、来源撤回不能回源绕过 | 真实模型一定遵守身份约束 |
| V13 | v1缺失标记、v2精确保存游标、SQLite事务失败注入、失败恢复分支回滚、中断账本恢复无网络 | 真实物理磁盘满/设备掉电 |
| V14 | 140 完整转写先 ACK 再淘汰 120 条 ring；延迟/失败阻止更新；原文分页hash校验 | 没有开启记忆的旧记录全量存在 |
| V15 | 原文 null/0/缺失、SQL/幂等/金额/单位/否定逐字匹配；截图原题与资产hash在原生路径保存 | 原图长期保存、真实OCR或视觉正确率 |
| V16 | MemoryStatus 不进入 chat/滚动签名；旧跟随/多窗口回归；所有记忆 IPC 拒绝展示窗 | 安装版多屏长会话 |
| V17 | 派生内容作为数据、短来源约束；精确原文候选不能升级本人身份；没有新工具执行器 | Prompt 注入对真实 LLM 的绝对免疫 |
| V18 | 既有失败终态/正文回归 + 三路上下文版本不重启；首个有效答案计时规则不变 | 新记忆真实首字 p50/p95 |
| V19 | 块引用依赖、原文回放、覆盖空洞、收益不足/变大拒绝，不递归只总结摘要 | 长期自然语言语义漂移率 |
| V20 | 10 项冻结资产/提示前缀摘要逐项比较；原 35 文字/12 视觉和口语输入不改 | 冻结验收题真实运行通过 |

## 复现

从源码根目录，使用既有依赖及本机工具链：

```sh
cargo test --locked -p interview-core --all-targets
cargo test --locked -p meetily --lib interview -- --nocapture
node scripts/test-public-source.cjs
node scripts/test-session-memory.cjs
node scripts/test-memory-settings.cjs
node scripts/test-background-ui.cjs
node scripts/test-record-revision.cjs
python3 -B scripts/test-toolkit-harness.py
python3 -B scripts/test-toolkit-preparation.py
python3 -B scripts/test-toolkit-consolidation.py
cargo run --locked -p interview-core --example context_measure
cargo run --locked -p interview-core --example memory_persistence_measure -- /new/path.sqlite
```

TypeScript 与 Next 生产构建单独在 frontend 执行。本次 production build 在隔离目录完成，node_modules 复用；没有生成/切换安装器。`scripts/build-memory-fixture.cjs <输出目录> <frontend/out>` 只为浏览器合成 UI 测试，调用固定 synthetic IPC，不连接模型，不能用于实际资料。

配对文件 `validation/context-compaction/cases.json` 含 3 开发/3 保留案例。A/B/C 的模型、来源和输出能力输入一致；C 只是确定性候选协议，不计成真实 AI2。保留组不参与调参。本轮没有更改该文件，未对真实口语质量、实际 token 或费用评分。

## C5 待运行清单（当前未授权）

1. 明确本批金额上限、服务商/模型、只用公开虚构数据或另行授权的会话范围；在软件填写独立 Key。旧 Harness 30 元不挪用。
2. 先 6 个固定案例 × A/B/C（18 次 AI1）；C 只对 4 个长会话案例各 1 次 AI2，短案例预期不需要整理。共 22 次基础调用，先按实际价表计算最坏输入/输出与总额；费用未确认前不运行。任何额外重试计入同一授权总额，不把失败排除分母。
3. 使用冻结来源/题目/能力配置；记录 AI1 有效首字和完成 p50/p95、AI2 时延、每次真实 prompt/completion/reasoning/cache/unknown、取消与失败。独立列 AI2 开销和整场开销；输入变短不等于省费。
4. 人工按固定原句核对否定、数字/单位、完成状态、本人贡献、追问指代与所有子问题。检查自然口语/短题简洁/长题覆盖；四角色离线反馈不替代人工保真。任一关键事实升级、丢条件或已撤回来源外发即不晋级。
5. 只有对应源码/模型门槛通过且获得切换许可后，才制作独立测试包，保护现有输入与采集状态，再验收停止/重开、恢复、多屏跟随、双路 ASR、视觉及 30 题/60 分钟。无付费验收不能自动开启 feature flag；真实双路音频、Windows 与数字人验收互不替代。

不承诺固定压缩比例或 25% 节省。若 C 相对 B 没有质量或整场成本收益，保留原文检索和默认关闭的压缩候选；原任务范围不因失败而删除。
