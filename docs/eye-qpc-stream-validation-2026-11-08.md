# AV-E5a 眼部 worker 原始 QPC 协议

基线 `8fcafedda038a1f6894ac53b34e515468b81a478`，独立分支
`feat/windows-eye-output-20261107`。接续[启动预检](eye-av-preflight-validation-2026-11-08.md)。
本轮完成原生 worker 的显式 QPC 输入/校正结果边界；Rust 监督器、预览和 joint owner
仍使用各自现有入口，尚未把新协议接成完整同步产品。

## 假设与决策价值

假设：原始采集 QPC 经推理和颜色转换后保持不变，处理完成时刻独立记录，旧毫秒路径
不变。验证决定能否继续将该边界接入共同时间线。停止条件为时钟混用、续期过期帧、
跨代次输出或降级旧协议；发现后保留失败并定位，不能放宽断言或用返回码代替像素检查。

已有视频输出 v2 表示**已经排期的 MF 帧**，不适合充当尚未经过音频调度的 worker
结果。本轮使用独立的 worker 输入 v2 和结果传输 v3；不会把处理完成时间当作呈现时间。

## 明确的协议边界

| 层 | 旧路径保留 | 新路径 |
| --- | --- | --- |
| 子进程参数 | `--eye-stream-v1` | `--eye-stream-v2`，不自动回退 |
| 输入头 | TXI1 / version 1，40 字节，末字段 captured_ms | TXI1 / version 2，独立 QpcInputHeader，40 字节，末字段 capture_qpc（100 ns） |
| Ready | TXER / version 1，24+236 字节 | TXER / version 2，24+236 字节，endpoint.version=3 |
| 校正输出 | TXV1 / version 1，kind 4 + NV12 | TXV1 / version 3，kind 8；头内旧时间字段必须为 0；载荷为 16 字节 CorrectedTiming + 1280×720 NV12 |
| 时间元数据 | 毫秒采集时间 | capture_qpc、completed_qpc 各 uint64；无呈现时间、无音频 |
| 最终报告 | JSON protocol 3 | JSON protocol 4，原计数/清理/设备未启用字段保留 |

认证、受限本地管道 ACL、PID/会话身份和临时 capability 沿用已有传输；capability
不进入日志、参数或公开快照。v3 控制头为 Hello/Ack/Begin/End（1/2/3/5），与版本一致。
kind 8 的 bytes 为 1,382,416，generation/id 非零且单调约束继续由会话/发送器承担；
没有 v2 的排期回执，写完只表示管道提交，不表示 MF/会议接收。现有 MF receiver
明确拒绝 endpoint v3；不能直接激活它来绕过同步调度。

Session 显式选择时钟域；QPC 模式下 captured_ms 必须为 0，输入不得提供完成时间。
完成 QPC 在 provider 返回后采样。排队、推理后、转换前后均以**原始采集时间**检查
250 ms 上限；拒绝零/未来/倒退/混合域。过期结果撤销输出，不补发原画面。
保留最多两个待处理帧和一个推理中帧、输出最多一个待发送帧和一个发送中帧。
停止沿用 generation/操作票据和独立 owner watch，迟到推理不能恢复已撤销输出。

接入方下一步必须严格核对 Ready/endpoint/report 版本、帧长/顺序、原采集和完成时刻，
并在**读取完整载荷后及交给 joint 前**重新检查年龄。传输耗时不重置租约。未完成这些
接线前，现有 Rust 控制器继续拒绝新 Ready，不对旧字段做单位推断。

## 实际验证

本机 Windows 11 Pro 10.0.26200、Ryzen 7 9700X、约 31.1 GiB 内存；RTX 4070 Ti SUPER
驱动 32.0.16.1088 仅只读核对。MSVC 19.29 / SDK 19041 / CMake 4.4.5 / Rust 1.99.1。
没有加载 NVIDIA SDK、启用 GPU 推理、物理采集、播放、设备注册或安装更新。

- 眼部 CTest 2/2：Session 155 个断言；实际子进程/管道的旧 8 组与新增 6 组行为检查。
  新路径三次独立重启，保留原始 QPC 和独立完成时间，验证 NV12 像素确实来自合成
  provider；检查未来/过期/重复输入、慢推理过期、跨版本拒绝、取消和无效完成时刻。
- 原生视频 CTest 5/5：旧预览/输出、跨进程 MF、排期 v2 及桥接回归通过。
- Rust `stream` 针对性回归 17/17：新编译的本轮 worker/视频 DLL 仍兼容旧监督器、
  预览、分路和 MF 样本路径；这不是新 QPC 的 Rust 接线验收。
- 新增 sink 边界断言后只重跑受影响的 stream CTest（1/1），最终全量 CI 另记。

证据目录：`artifacts/2026-11-08/eye-qpc-001/`，含实验定义、环境、构建日志、
`ctest-eye-1.log`、`ctest-eye-final.log`、`ctest-video-1.log`、`rust-stream-1.log`、
受保护工作树/原安装进程记录和 SHA-256 清单。所有输入均为内存纯色，无私人媒体。
本轮本机未出现失败重跑；第二次 stream 检查由新增断言触发，不覆盖首轮日志。

## 未通过的门槛

新 QPC Rust receiver/共同 owner、摄像头源时间映射和本人原声采集、实际 SDK、外部
排空/安全恢复、虚拟音频设备、主窗状态、安装版和会议接收端仍待完成。合成 provider
人为等待只证明元数据行为，不能代表眼部自然度、GPU 性能或 P95 ≤100 ms。
本轮没有安装器/发布资产/第三方二进制，商业分发许可状态不变；不标为稳定版。
