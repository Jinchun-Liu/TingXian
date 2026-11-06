# Windows 视频设备生命周期验收

代码提交 `ddada3c6206bc407c0f2904e0a55a4d04f9fd4a8`，分支 `feat/windows-eye-local-20261106`。
这是独立视频组件续作；没有更改基础安装包、前端、资料格式、应用标识或 Mac 0.6.5 UI 基准。

## 实现与验证决定

新增显式设备控制层与 Windows 后端：只允许 Session / CurrentUser，启动前核对 64 位系统注册，拒绝用户级同名 COM 覆盖；同用户输出用进程生命周期内的内核对象协调。需要用户显式启动，并在专用 MTA 执行设备调用；构造和恢复没有启动副作用。

停止先撤销帧生产者，再关闭设备。启动中收到停止请求时，迟到的启动结果进入清理；Stop 失败仍尝试 Shutdown。原操作与清理错误分别记录，Shutdown 失败保留对象供明确重试，不覆盖尚未释放的设备。实际主应用/采集 worker 尚未接入，析构只能提供尽力清理，不能代替显式停止结果。

假设：取消、部分启动失败和控制者退出不会留下获准发送的旧帧。决策价值：判断能否进入主控制窗与独立安装组件接入。停止条件：错误启动、丢失原错误、覆盖清理失败的对象、停止后旧帧继续有效、错误清理顺序或竞态。详细契约及官方接口依据见 [SESSION.md](../video/windows/SESSION.md)。

## 分层结果

[独立 CI 42211608390](https://github.com/Jinchun-Liu/TingXian/actions/runs/42211608390) 与本机各完成以下检查：

| 检查 | 结果 | 证据范围 |
| --- | --- | --- |
| 原生媒体源 | 15 组通过 | 加载本次 DLL，实际 COM/MF 合成样本 |
| 跨进程通道 | 10 组通过 | 两个普通用户进程传输合成 NV12 帧 |
| 新生命周期 | 13 组通过 | OS 设备生命周期使用故障注入；另外验证真实内核对象、管道撤销及中性像素 |
| 只读前置诊断 | CI 与本机均正常完成 | 导出函数/注册项检查，不调用创建或支持查询，不触发权限请求 |

CI 为 Windows Server 2022；目标机为 Windows 11 `10.0.26200.0`，当前测试进程非管理员。四个本机程序均退出 0。没有残留测试进程，原听弦 PID 76724、2026-11-05 21:11:44 启动的进程继续运行。旧归档工作树干净，仍为 `2a80d7bf280d75d67ab985fe6cd10f10b095f5e9`。

13 组覆盖：内核占位释放；默认空闲与三次重启/重复调用；初始化失败；Start 失败与显式恢复；Stop 失败仍清理；Shutdown 失败阻止替换并允许重试；双重错误；非法端点；析构；prepare 阻塞时取消；Start 阻塞时取消；生产后端拒绝未初始化 COM/STA；实际管道在模拟 OS Stop 失败时仍断开并输出中性帧。

内核占位测试在同进程创建两个句柄，未声称验证了跨登录会话或实际多开应用。设备 OS 操作是注入边界，不能用这些通过结果代替真实注册、设备移除或 FrameServer 服务访问。停止竞态测试使用同步屏障，不证明 PRD 的 500 ms/1 s 实机时延。

## 本机诊断结果

`tingxian-video-device-probe.exe --probe` 返回：

```json
{"api_exports_present":true,"machine_registration_present":false,"user_registration_conflict":false,"api_error":"0x00000000","registration_error":"0x80040154","device_created":false,"physical_camera_opened":false,"runtime_verified":false}
```

因此下一步需要制作并验证独立媒体源安装组件。存在 API 导出不等于设备/权限/SDK 已可用。CI 的系统返回 API 导出缺失 `0x8007007f`，注册同样缺失；诊断正常报告差异，没有把 CI 系统当作 Windows 11 设备验收。动态解析 API 使诊断在不支持的平台上仍能启动。

## 产物、失败恢复与待测

最终 artifact `13500050337`：ZIP **690788 bytes**，SHA-256 **728ea9de5e67c14dac11df293cd399424b4dd58a9fa5b8b4d41d7cb03b72c5e8**。
包内有媒体源 DLL、三个测试 EXE、只读诊断 EXE、控制层静态库、原始 CTest 日志和许可。它不是安装器或稳定版本。

本地证据在 `artifacts/2026-11-06/video-session-ddada3c/`。两次下载因 GitHub EOF 失败，原失败目录及零字节 ZIP 均保留；独立 `download-http1/` 使用当前下载进程的 HTTP/1 设置取得文件，并核对源码 SHA、GitHub artifact 大小/摘要及逐文件摘要。单次成功不足以认定 HTTP/2 是网络故障根因，未修改系统代理或防护。四份本机日志、`local-results.json` 和成功的 CI 元数据在本轮根目录。

尚未安装/注册虚拟设备、执行真实 MFCreateVirtualCamera、验证 LocalService/FrameServer、连接主应用或加载眼部 SDK。没有摄像头、音频或 AI 采集/外发。普通安装的 360 DLL 写入阻塞、SDK x64 组件与许可、实际图像质量、性能、安装版 UI、真实会议仍为独立未通过门槛。旧数字人、脸部模型、TTS 和代理继续冻结。
