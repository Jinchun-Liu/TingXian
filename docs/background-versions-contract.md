# 背景来源版本与增量整理

本契约在 Mac 分支实现。共享 Rust 核心、前端和 Python Harness 可供 Windows 合并；Windows 机器和两种 Agent 的真实回导尚未验收。

## 数据边界

保持 `meetily-background-v1` / `meetily-background-v2` 与现有 SQLite schema 兼容。在 `background_library` 增加 `revision/<source_id>/<extraction_hash>` 键，值为 `document`、`index_version`、`recorded_at`。`source_id` 是来源 ID，`document.hash` 是原文件摘要，`extraction_hash` 是规范序列化 Doc 的摘要，三者不能混用。

归档与资料索引同一事务写入。相同提取摘要只保存一次；旧应用导入资料没有可信保存时间时为 null。`index_version` 用于排序，不表示事实生效时间。64 MiB 归档上限超出即回滚，不清理旧版本。现有索引清理备份包含整张 `background_library`，因此保留历史提取；备份验证失败不得清库。

历史只保存提取记录，不替代源文件备份；不进入 chunks、FTS、事实卡或普通模型输入。既有资料核对状态仍绑定当前提取摘要，摘要改变后确认失效。

## 主窗口只读命令

| 命令 | 参数 | 结果与隔离 |
| --- | --- | --- |
| `background_changes` | `draft` | 来源 ID、文件名、new/source_changed/extraction_changed/unchanged、其他相同原文的来源数量与最多3个名称；核对草稿及资料库版本 |
| `background_preview` | `draft`, `document`, `offset` | 本次 rows 与 previous，任一侧还有内容时 more=true；20条一页，不修改资料 |
| `background_versions` | `source`, `expected`, `offset` | 20个版本、more；每版 revision/name/source_hash/recorded_at/count/current；expected 为当前提取摘要 |
| `background_version_records` | `source`, `expected`, `revision`, `offset` | 20条引文、more、current；校验当前来源与归档摘要，旧版本不可应用或自动恢复 |

所有命令仅主窗口可调用，不持有 Key，不发模型请求。前端来源改变、翻页、卸载后丢弃旧请求结果；会话锁定时不新增版本查看操作。

## 模型整理输入

当前允许外发的片段增加 `source_hash` 和 `source_status`；后者含确认、有效期及核对时间。不存在历史版本自动授权。用户本次选择整理的文件仍按原有明确发送流程处理。

Harness 2.0.1 按 `(kind, quote)` 合并完全相同引文，`sources` 保留各自来源/定位/状态。新选择文件标作 current，库中未替换资料标作 indexed，被本次更新替换的条目标作 superseded。current 不表示本人已确认，多个来源也不是独立证明。保留日期、职责、完成状态矛盾；生成建议和事实确认仍分开。

来源数组超单批预算时拆组，不丢来源。语义相似但文字不同不在本机自动合并。汇总提示变更影响任务签名，旧规则未完成任务保留但需新建；交换包格式无需升版。

## 验证边界

核心测试覆盖重复导入、确认失效、旧库升级、同内容副本、授权隔离、重开数据库历史可见、旧哈希拒绝、归档上限事务回滚。原生合成清理测试包含历史归档。Python 覆盖跨批引用合并、来源时效/冲突标记和预算拆组；前端覆盖迟到隔离、显式查看、分页与未知旧时间。

未验证：安装版异常中断、真实模型对矛盾资料的判断质量、真实 Codex/Claude Code 推理和导回、Windows 原生 UI/归档。
