# Windows worker 所有权与停止检查

日期：2026-11-06 21:28—21:48（UTC+8）。分支 `feat/windows-eye-local-20261106`。
实现 `c42119285c0dfc346a64fb97a84b57f529780b1e`，最终验证源码
`0c3695d3118e00c5490feba55f460273e423c2fb`。

## 改动与决定

为主应用的异步 worker 接入补上停止先到、旧启动任务后执行的竞态：
如果只在 `CameraSession::start` 执行时读取取消代次，排队期间已经发生的停止会被遗漏。
新增 `start_if_current`，在派发前固定票据；`WorkerOwnerGuard` 在开始监视前取得票据，
只允许一次启动。原同步入口保留；未来应用启动必须经过这个一次性边界。

guard 监视明确传入的主进程句柄与匿名停止事件。主进程退出、停止、监视失败或
guard 关闭均先撤销生产者权限；慢 OS 调用的迟到成功不能恢复有效会话。
设备 COM 清理由 worker 自己的 MTA 显式执行并核对结果，监视线程不越线程调用 COM。
事件创建错误在后续 Windows 调用前保留，避免失败被成功调用的 last-error 覆盖。

假设、决策价值与停止条件见 [worker 契约](../video/windows/WORKER.md)：
只要出现意外启动、未撤销输出、遗留测试子进程或清理结果错误就不接受该实现。
本次决定只批准继续接入主应用，不批准眼部功能发布。

## 产物、环境与范围

| 项目 | 本轮记录 |
| --- | --- |
| 目标系统 | Windows 11 `10.0.26200.0`，普通用户，D 盘中文路径 |
| CI 原生 | [42227075419](https://github.com/Jinchun-Liu/TingXian/actions/runs/42227075419)，Windows 2022，6 个 CTest / 16+12+13+8 组通过 |
| CI 安装 | [42227075392](https://github.com/Jinchun-Liu/TingXian/actions/runs/42227075392)，7 组部署及 5 组升级/回滚通过 |
| 原生 artifact | ID `13506097253`，1145696 bytes |
| ZIP SHA-256 | `e50ec873119b57be52e3dd78ec9f207e7fbcdaab9b6720c1bf5103984d4d0f50`，与 GitHub digest 一致 |
| owner 测试 EXE | 257024 bytes；`e7dd1e007bb225970fbba07efcabfad0844f200eb235f3513db6fbe784a3b641` |
| session 测试 EXE | 345088 bytes；`cf955dcfdf3f19e47c625da3c5bfd1f141668dc59d82747b61c253aece53da00` |
| 已安装组件 | 保持 0.1.2 / `084dae5`；本轮没有重新安装 MSI |
| 安装 MSI 历史摘要 | 151552 bytes；`ae61e657324370b32abf1b7c3725f1f58c42de08896d482922940c49542865a8` |
| 安装 DLL 本轮复核 | `ee541449f2eb41a8bbf58c5f96341a0ead302f611c432c677954855fb840f5d3`，前后相同 |

CPU/GPU/驱动的同机基线见[上一轮设备记录](video-device-validation-2026-11-06.md)。
本轮未启用物理摄像头、音频、SDK、GPU 推理或 AI 外发，也没有创建虚拟设备。
现有基础应用的 PID、启动时间及路径前后一致，用户资料未操作，未改防护。

## 本机步骤与实际结果

1. 核对源码提交、CI 成功及 artifact SHA-256 后解包到源码外 D 盘目录。
2. 普通用户运行 owner 测试。真正创建两个受限句柄继承的隐藏测试进程，分别扮演
   主控制进程和 worker；设备 OS 后端使用故障注入。停止、主进程退出、预先停止、
   预先退出、排队后停止、慢 Start 迟到成功六种进程情况，以及无效句柄和取消票据
   两种本地情况，共 **8 组通过、退出 0、stderr 为空**。
3. 运行改动涉及的 session 回归：**13 组通过、退出 0、stderr 为空**；包括真实命名
   管道关闭后的中性像素，以及 OS Stop 失败时的权限撤销。OS 设备生命周期仍为注入。
4. 验收后检查测试进程数量为 0、原应用未重启、安装 DLL 摘要未变。

没有为通过而放宽断言。第一次 CLI 下载 ZIP EOF，第二次元数据 EOF，连接器文件
直接传输也失败；保留原文件/错误，在新目录经 GitHub 连接器和本机已有代理下载，
与最初 GitHub 元数据的完整 ZIP 摘要一致后才执行。没有将下载失败算成测试失败或
以未验证二进制替换。

证据在源码外 `artifacts/2026-11-06/video-worker-owner-0c3695d/`：
`before-and-decision.json`、`artifact-metadata.json`、`artifact-integrity.json`、
`owner.stdout.log`、`session.stdout.log` 与对应 stderr/result、`local-results.json`、
`ci-installer.log`、`ci-summary.json`、`acceptance.json`，以及全部下载尝试。

## 尚未通过与下一步

这是原生库和进程控制验收，**Tauri 尚未调用这个 guard**，没有交付眼部 worker 或
新基础安装版。下一步需要将主窗独占权限、可验证 worker 启动、退出归档握手和卡住
worker 的处理策略接起来，再用已授权纯合成虚拟设备验证主进程退出后的实际移除。
当前不能声称满足 PRD 的采集停止时间或会议帧率/延迟。

NGC 当前组织订阅为 0。学校邮箱进入 NVIDIA 新账户创建页后，用户说明暂时无法
注册，已暂停，未提交评估申请或创建 Key。真实 Windows x64 SDK/模型/许可、GPU
加载、眼部质量、真实摄像头、会议和 30/60 分钟性能仍待完成。360 防护下基础安装器
问题仍是原有独立缺口；视频 MSI 与本次测试不替代其验收。版本继续保持实验/测试定位。
