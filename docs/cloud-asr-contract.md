# 云转写适配与双平台合并约定

实现入口：`interview-core::cloud_asr` 负责协议与 WebSocket；原生 `interview_cloud_asr` 负责音频帧、设备来源、取消及 Tauri 事件。桌面适配不依赖 macOS API，Windows 可复用现有 WASAPI 管线。

## 配置与权限

`cloud_asr` 设置保存 `mode: local | aliyun`、`workspace_id`、`silence_ms`（200–6000）。旧数据库默认 local。Key 不持久化、不返回窗口、不放入恢复草稿；主窗口 `interview_cloud_asr_get/save` 仅在无采集/会话时允许修改配置。

`interview_audio_start` 新增可选 `cloud_asr_allowed`，缺省 false，旧调用仍为本地。文字模型授权与声音外发授权独立。展示窗口不保存 Key、不启动音频或连接供应商。术语外发只包含用户配置的术语，最长 400 字，不注入未说出的 AI 建议。

## 流与事件

每个来源独立 UUID task。云端相同 sentence_id 不表示相同说话人，适配层生成应用级唯一 utterance_id。保留现有 `source: System | Microphone`，这是采集来源，不是多人声纹身份。

| 阶段 | 事件与消费方行为 |
| --- | --- |
| task-started | 建连就绪，可开始送音频 |
| sentence_end=false | transcript-update，is_partial=true；仅字幕和原始记录，无问答/滚动 |
| sentence_end=true | transcript-update，is_partial=false、replaces_utterance=true；再发 interview-utterance-end(success=true) |
| 断流/取消的未完成句 | interview-utterance-end(success=false)；保留 partial 为中断原文，不合成为完整题目 |
| task-finished | 仅结束对应来源，不复用 task 或重放音频 |

事件携带 native_session_id、utterance_id；麦克风另带 microphone_epoch。前端还绑定 capture_session。暂停增加云 capture epoch，恢复新建 task；关闭麦克风撤销旧 gate，旧结果无效。System 的 final/end 通过既有多人路由和授权后才排入回答队列，Microphone 永不触发回答。

PCM 帧及其内存 admission lease 在网络发送完成前不可释放计数。发送上限、连接/发送/结束超时明确报错，断流不静默改来源、不补传、不自动改本地。用户停止后可在设置里切回本地 Whisper。

`interview-cloud-asr-usage.task_billed_seconds` 是供应商任务累计值，可能缺失；不要按每次句子事件求和。未来费用台账需要增加供应商 task 标识和去重，不拿该字段代替实际发言时长。

## 当前证据边界

协议测试使用本机合成 WebSocket，覆盖 ready-before-audio、partial/final、重复终稿、结束收尾、断流和取消。页面测试覆盖独立授权、本人不答和旧字幕隔离。真实百炼连通、双通道长会话、模型准确性和配对 P50/P95 仍待账号/设备实测。

官方协议核对于 2026-11-06：[实时接口](https://help.aliyun.com/zh/model-studio/fun-asr-realtime-websocket-api)、[客户端事件](https://help.aliyun.cn/zh/model-studio/fun-asr-client-events)、[服务端事件](https://help.aliyun.com/zh/model-studio/fun-asr-server-events)。当前北京区地址按 Workspace ID 派生为 `wss://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference`。如服务商变更地址或协议，先核对官方文档，不能静默改为任意外发地址。
