# AV-E6a 原始麦克风输入边界

基线 `a0b941d63b586be2099b6968336780d7a2f18824`，沿用独立
`feat/windows-eye-output-20261107`；上一轮媒体 Host/CI 为实际进展，稳定版目标仍未达成。

## 决策与实现

假设：独立 WASAPI 输入组件能保留明确选择的本人麦克风、设备首帧位置、原始 QPC
及状态标记，并在同一原生线程成对获取/释放系统缓冲。决策价值：为共同媒体 Host
补实际输入边界，不将旧转写回调的会话经过时间冒充采集时间。
停止条件：必须开启未授权的设备，错源/伪造时间、缓冲无界、遗漏释放或隐藏清理失败。
影响：输入后端、原生 ABI、来源/时钟准入、回归与后续主进程接入。

新增可选 `TingXianAudioCapture` 原生组件：

- 默认不初始化或启动捕获。准备需要显式许可、非零代次和选中的端点 ID；检查与
  真正准备分开，准备时重新查询系统。拒绝输出方向、已选回采端点、已识别虚拟线缆、
  非活动/重复 ID 及未知 form factor；只接受 microphone/headset/handset 元数据。
  不靠友好名称认定来源，不使用系统默认设备，不自动切换。元数据不能证明真人声音或无声学回声。
- 向共享音频引擎请求 48 kHz、单声道 float 及必要格式转换；不改硬件格式、系统默认
  设备/音量，不启用 loopback。该请求不等于各硬件都支持或转换质量已经通过。
- 一次最多 4800 帧（100 ms），固定存储，无内部增长队列；保留真实 PCM、首帧
  设备位置和 100 ns QPC。整包验证并释放后才交给调用方，错误/空包不会重放上一包。
- 拒绝未来/过期/倒退时间、缺失时钟、后续设备位置不连续、未知标记、非有限样本及
  时间戳错误；相邻包相对标称时长残差上限 1 ms、年龄上限 250 ms，均为实验界限。
  首包没有前驱，其 DATA_DISCONTINUITY 作为新流边界保留原标记并计数；后续同标记故障。
- SILENT 包允许空系统指针并输出明确的零样本。错误路径也释放已取得的非空系统包，
  原始释放/停止/重置 HRESULT 分开保留，清理未确认不能报告正常停止。
- 线程归属、代次、格式/容量检查覆盖原生 ABI；停止后同一 Session 不重启。三次新
  生命周期的合成回归验证无旧数据继承，不能代替实体设备重启验收。

依据微软官方 [GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
契约，返回时间对应包中第一帧，已换算为 100 ns；获取与释放必须在同一线程。
[共享流转换标志](https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants)
提供客户端格式转换。[Stop](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-stop)
和 [Reset](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-reset)
明确允许 S_FALSE 表示先前已完成；代码只接受其明确的 S_OK/S_FALSE，保留原值，不接受任意正数。

## 验证与证据边界

证据目录：源码外 `artifacts/2026-11-08/audio-input-001/`，D 盘。
本机使用既有 VS 2019/MSVC 19.29、SDK 19041 的 `cl /Zs /W4 /WX`，只做语法/类型检查，
不生成或执行测试 EXE。`syntax-1.log` 因 GetBuffer 的 DWORD 指针类型不一致失败；
改用原生 DWORD 接收标记，`syntax-2.log`、`syntax-final.log` 通过，失败日志保留。
没有重跑/恢复上一轮被 360 清除的 Rust 测试 EXE，没有修改防护。

新增 CTest 内存后端覆盖 PCM/QPC 原值、首包/后续边界、静音、容量、未来/旧时间、
错误来源/权限/线程/代次、设备失效、缓冲释放、清理错误及三次独立生命周期。
另一个生产 DLL ABI 检查只执行默认状态、拒绝许可、未知端点检查和错误线程销毁拒绝；
它不对有效设备调用准备/启动。

功能提交 `16939c6778f405292a6dd6ee8f74d4991e3433b2` 已原样推送。
[原生 CI 42451324716](https://github.com/Jinchun-Liu/TingXian/actions/runs/42451324716)
完整成功：127 项 Rust 回归，9 项视频、2 项眼部、5 项音频 CTest，以及 1673 个
公开源码文件检查通过。音频 CTest 的 `LastTest.log` 确认新增内存输入测试为
214 条断言通过，生产 DLL 的拒绝许可/未知端点/线程归属检查通过，未初始化或启动
捕获。其余三个音频 CTest 为既有输出内存、端点只读和未知配对拒绝测试。

CI 工件 `13605796650` 在 D 盘下载后实测 4473656 bytes，SHA-256：
`599082349dc2a040d9d45d22d3078a962787169261663337fcd0e01a7dc56fd0`，与平台摘要一致。
只提取 4 个文本日志；没有提取或运行其中的 EXE/DLL。证据为 `ci-run.json`、
`ci-job.log`、`ci-artifact-integrity.json` 和 `ci-text-logs/`。这些是独立 CI
证据，不解除本机 360 拦截；本轮没有构建或验证新主应用安装包。

结束检查：本机 Windows 11 Pro 26200；教育、眼部原分支及数字人归档工作树均保持
原 HEAD 且干净，无本轮修改。当前功能分支与远端同步；本轮文档收口提交另见 Git 历史。

本轮没有物理采集/播放、SDK 推理、虚拟设备注册或应用升级；新组件没有随主应用
安装器打包。Rust 输入绑定、原声批次到 Host 的有界接线和共同停止仍待完成，不能
把此组件存在报告成已接通原始音画链路。真实格式转换、USB/蓝牙、权限拒绝/拔插、
时钟精度、正常防护下实机安装、接收端偏移与长会议仍未验收。
实时辅助旧采集、教育及 Mac 分支保留；完整数字人/TTS/换脸继续冻结。
