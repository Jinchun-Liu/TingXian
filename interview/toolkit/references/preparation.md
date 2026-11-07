# 岗位视图与项目故事卡

这是离线 Lead 的来源化整理流程。输入是已经由背景工具校验的 v1/v2 包、明确的资料库标识、目标岗位要求，以及当前人工修订/排除清单。原资料只读。程序不读取应用数据库、不请求模型、不写回个人事实，也不把引用存在当成语义已证实。

## 实际入口

在已选择的项目里，用背景执行组件的 plan/read/pack 核对当前文件与引文，再准备 task.json：

```json
{"role":"测试开发实习生","organization":"目标团队或留空","requirements":["契约测试与业务语义","真实线上故障处理"]}
```

从工具包根目录执行：

```sh
python3 toolkit/harness.py preparation-plan --bundle background.meetily.json --library-id selected-owner --task task.json
python3 toolkit/harness.py preparation-input --bundle background.meetily.json --library-id selected-owner --task task.json --part 1
python3 toolkit/harness.py preparation-collect --bundle background.meetily.json --library-id selected-owner --task task.json --proposals responses.json --execution-mode host_model
```

有人工修订或排除时，每个命令都传相同的 `--revisions revisions.json --excluded excluded.json`；不能用遗漏排除参数来“恢复”旧输出。修订文件沿用 archive 的记录ID，需 claim、basis、revision_id，可附 activity=unknown/planned/in_progress/completed。第二次修订必须引用上次输出的新记录ID，指向已被取代记录会拒绝。排除数组填来源 document.id。

默认只装配已验证方法；当前方法仍为候选，因此默认不追加。开发对照可在所有命令加 `--preview-methods`，只选岗位匹配/贡献/状态/项目证据相关的 CV 与面经规则，不加载排版、ATS、32方向全文或三角色说明。方法与例外进入预算、规则来源留在离线 manifest；开关变化使输入 hash 改变，不能混用旧响应。

plan 列出分块编号、每块 input_hash、记录范围、保守输入估计与输出/安全预留。input 一次只返回指定分块的 messages；让宿主高智能模型执行其内容，将响应对象按块保存在 responses.json 数组。hash 原样复制，不自行计算或改成新 hash。未改动且哈希仍在当前计划中的成功响应可复用；新/变化块才需要重新处理。分块按来源标识和原文位置排序；记录长度、增删造成边界变化时，相邻块也可能需要重新处理，不承诺每改一份文件只花一次调用。遗漏块拒绝收集，旧块明确列入 ignored_stale_inputs，不参与新产物。

默认输入上限32768（UTF-8字节保守上界），输出预留8192、安全预留4096。`--context-tokens`取实际宿主模型窗口；工具再与262144取较小值。`--input-cap`和`--output-reserve`可按模型调整，但每个命令必须一致。8192只是可调默认，不代表高推理模型统一适用。单条或任务无法装入时明确拒绝，不能截断来源/岗位要求凑预算。输入总和是多块累计估计，不能当单次窗口或实际usage。

## 模型响应

每块只输出以下字段；所有任务 requirement.id 均需回应，无证据必须留 gap。

```json
{
  "format":"preparation-part-v1",
  "input_hash":"从输入原样复制的64位摘要",
  "requirements":[{
    "id":"req-1",
    "matches":[{"record_id":"本块个人记录ID","relevance":"direct","reason":"与岗位要求相关的依据及限制"}],
    "references":[{"record_id":"本块参考资料ID","purpose":"仅供补知识，不能证明个人经历"}],
    "gaps":[]
  }],
  "stories":[{
    "title":"项目或经历名称",
    "activity":"unknown",
    "sections":{
      "context":[],"task":[],
      "action":[{"text":"保留限制的本人行动表述","record_ids":["本块个人记录ID"]}],
      "result":[],"learning":[]
    },
    "missing":{"context":"尚缺情境依据","task":"尚缺本人任务","result":"尚缺验证结果","learning":"需本人补充复盘"}
  }]
}
```

引用由程序回填原文、来源哈希、定位和本人修订。模型不得改引文或把提取注解当原文。matches/故事片段仅允许 personal/evidence；practice/job/technical 留在 references。原始引文未结构化标注完成状态时必须 unknown，待本人明确修订后才能据此更新。不同来源重复同一原文不是多次独立验证。

收集器按岗位要求合并各块，direct在supporting之前，但不冒称完成跨块语义排序。故事保持分块来源；同一项目跨块重复、局部gap被其他块补足以及职责冲突，由Lead与三名reviewer核对，不能静默合并姓名相同的不同项目。未选记录仍保留于完整档案。

## 核对与回导边界

`preparation-collect ... --output-format markdown` 输出可读稿；`--output-format team` 输出共享 team-audit 可用的 candidate。三名reviewer接收同一份 candidate 及其 canonical hash，按照 execution.md 独立核对。candidate.id来自产物摘要，review.candidate_hash则必须是完整candidate的摘要，二者不混用。收集后的隐藏改写会使旧评审失效。程序并未执行三角色模型，也未替宿主统计无法取得的费用。

本产物为 `tingxian-preparation-v1` 候选，不是 `meetily-background-v1/v2` 交换包；当前桌面“导入 Agent 结果”仍只接收原背景格式。不要把故事文本塞回原文或已确认事实来绕过这一边界。后续桌面来源核对与独立应用入口尚待接入。应用侧必须再校验自己的源版本、修订、时效与排除状态，不能只信可移植工具声称 current。

## 无模型示例

包内 examples/preparation-bundle.fixture.json、preparation-task.json、preparation-responses.fixture.json 是虚构的手工测试夹具。可将前三个命令的参数指向这些文件，library-id 使用 synthetic-owner，并用 --execution-mode fixture 收集。这个示例只验证命令与格式，不代表真实模型运行或三角色评审通过。
