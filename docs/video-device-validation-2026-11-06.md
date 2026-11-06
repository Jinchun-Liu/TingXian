# Windows 合成虚拟设备验证（2026-11-06）

目标机 Windows 11 `10.0.26200.0`，Ryzen 7 9700X（8C/16T）、约 32 GB 内存、RTX 4070 Ti SUPER（驱动 32.0.16.1088）；普通用户进程。基础应用与用户资料未改动。用户已单独允许纯合成虚拟设备测试，仍未授权物理摄像头、音频或真实 AI 外发。

假设：已安装媒体源能通过 Windows FrameServer 接收本机生成的 NV12 帧，生产者停止后中性化，Session Shutdown 后移除并可重启。决策价值：是否可继续主应用接入。首个 API/像素/时限/清理错误即停止，保留失败；三次循环只在上一轮成功后推进。测试不加载眼部 SDK、不记录图像、不改变防护。

## 首次失败与定位

- 已安装组件 `9eb55d3` / 0.1.0，MSI 151552 bytes，SHA-256 `d95cf62173f9dea8dc7848fbf2ad7f58d08e5078f5be0140132ed837a67c14af`；安装及 19 条断言见独立安装记录。
- `4e795c5` 工具于 20:44 首次运行：生产者创建成功，session_start 返回 `0xc00d3e85`，清理返回 0；没有读取帧，未运行第二/第三轮。
- `2aba3f8` 诊断工具于 20:52 复现：`create_virtual_camera` 和配置写入成功，失败确切位于 `start_virtual_camera`。`GetCurrentPackageFullName` 返回 15700，进程无 package identity；宿主打包身份假设不支持。
- 本机已启用的 FrameServer 事件记录同一时间的共享源初始化失败。原始本地记录保留设备元数据，不上传；未仅凭 HRESULT 推断为安全软件拦截。
- 后续只读检查未发现听弦设备或测试进程残留，基础应用 PID/启动时间不变。MFTrace 尝试没有生成跟踪文件，不作为底层调用链证据。

工具来源：[4e native CI](https://github.com/Jinchun-Liu/TingXian/actions/runs/42219564091)、[2aba native CI](https://github.com/Jinchun-Liu/TingXian/actions/runs/42220285214) 均通过普通 CTest；实际设备模式从未在这些 CI 中运行。2aba artifact ZIP 862559 bytes，SHA-256 `a628d040383e5e626225ce25ac0f8e1f424ee5d121c3d45bce444b241191242f`；测试 EXE SHA-256 `51eb2a2ad05d7b904c196ca0367783c0fa346e8dda13e9431c228178ffa08b81`。

## 生命周期修复与升级验证

源码 `745d16f` 移除 `Activation` 析构中隐式调用源 Shutdown 的行为。媒体源返回给调用者后，调用者仍持有它；释放激活器不能提前终止其工作。显式 `ShutdownObject` 和源 Shutdown 的清理行为保持。依据：[微软 ActivateObject 生命周期](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfactivate-activateobject)及 [ShutdownObject 责任](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfactivate-shutdownobject)。

新增回归检查“释放激活器后源仍可用，显式 Shutdown 后才拒绝”；源检查由 15 增至 16 组。注册 COM 工具也覆盖这一路径。独立组件升至 0.1.1、更换 ProductCode、保留 UpgradeCode 与文件组件标识。[原生 CI 42221693360](https://github.com/Jinchun-Liu/TingXian/actions/runs/42221693360) 与[安装 CI 42221693379](https://github.com/Jinchun-Liu/TingXian/actions/runs/42221693379) 均通过。后者包含原 7 组部署检查及新增 5 组：旧 DLL 准确复现 MF_E_SHUTDOWN、失败升级恢复旧版、成功升级修复、降级拒绝、升级后卸载保留未知合成资料。

21:00 本机用同一套新测试对照：已安装旧 DLL 的 COM 及源测试均准确在释放激活器后返回 `0xc00d3e85`；新 DLL 的 16 组源测试通过。无设备创建。证据 `video-device-745d16f/lifetime-differential.json`。

## 0.1.1 本机升级与下一处失败

MSI `TingXian-VideoSource-0.1.1-experimental-x64.msi`，151552 bytes，SHA-256 `2cc6a64267c43042dde0e95972d95bcf690d4e1a2d0ec551127b04cbbd6088d1`。用户本人允许 UAC 后，21:02:40 升级退出 0，无重启要求。21:03 普通用户完成 20 条安装断言，包含旧 ProductCode 已移除、新 ProductCode 已安装、独立清单/文件摘要、保护 ACL、HKLM 注册与修复后的 COM 生命周期。基础应用进程/启动时间不变，360FsFlt 与 360tray 仍运行，未改变防护。证据 `video-local-upgrade-745d16f/verification.json`。

随后同范围纯合成设备测试：Start 成功、获得自身 symbolic link、枚举到自身设备、共享预览/异步 reader/NV12 720p30 格式设置均成功。确认首次 Start 故障已由生命周期修复解决。下一项“三帧不同合成画面”失败 `0x8000ffff`；当轮停止，Session 清理和精确设备移除检查均返回 0，未继续第二/第三轮。该状态不等于实际收到了生产者画面。`39bddab` 增加生产者握手阶段、提交计数和预览中性帧计数以定位跨进程链路；不记录像素或能力凭据。

## 0.1.2 传帧修复与测试工具清理修正

进一步的 `39bddab` 诊断复现中，握手/Begin 已成功，首帧写入返回 broken pipe (`0x8007006d`)，成功提交 0、预览读取 321 个中性帧、0 个有效生产者帧，清理/移除成功。未将中性帧当成传帧通过。`084dae5` / 0.1.2 将初始化中的连接移至实际 Source Start；增加“纯格式探测不消耗连接”和“显式停止撤销尚未使用的配置”回归。本机同一测试对照旧 DLL 准确复现提前连接，新 DLL 12 组 IPC 全通过。

[原生 CI 42223228839](https://github.com/Jinchun-Liu/TingXian/actions/runs/42223228839) 的 16 组源、12 组 IPC、13 组生命周期及两项只读/描述检查通过。[安装 CI 42223228893](https://github.com/Jinchun-Liu/TingXian/actions/runs/42223228893) 的 7 组部署与 5 组 0.1.1→0.1.2 差异/回滚/升级检查通过。

0.1.2 MSI 151552 bytes，SHA-256 `ae61e657324370b32abf1b7c3725f1f58c42de08896d482922940c49542865a8`，构建源码 `084dae59e3ee150571e979211e4e4282826881b7`。用户批准 UAC 后，21:13:24 目标机升级退出 0；普通用户 21 条安装断言通过。安装 DLL 188416 bytes，SHA-256 `ee541449f2eb41a8bbf58c5f96341a0ead302f611c432c677954855fb840f5d3`。

21:14 实际设备首轮：提交 4 帧，读到 3 种完整合成像素，生产者错误为 0，停止后 1 秒内读到中性画面，Session 清理与精确设备移除成功。此轮仍判失败：工具释放 Source Reader 时 Windows 已关闭预览源，后续显式关闭再次返回 `MF_E_SHUTDOWN`。原错误记录完整保留，没有忽略错误或降低清理断言。

`c4a8a8e` 仅修正测试工具，设置 `MF_SOURCE_READER_DISCONNECT_MEDIASOURCE_ON_SHUTDOWN=TRUE`，由测试作用域负责唯一一次显式关闭预览，并记录其 HRESULT。[微软官方约定](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-source-reader-disconnect-mediasource-on-shutdown)明确此属性的所有权关系。已安装媒体源保持 084dae5，无需再次升级。

## 最终纯合成设备复验：通过

`c4a8a8e` 的[原生 CI 42224035339](https://github.com/Jinchun-Liu/TingXian/actions/runs/42224035339)及[安装 CI 42224035233](https://github.com/Jinchun-Liu/TingXian/actions/runs/42224035233)均通过。测试工具 artifact 994832 bytes，ZIP SHA-256 `bf3ba178f1edc6c14910421afdcbf574d79f405def1e3d306290985314121442`；EXE SHA-256 `411b382b92f3f3d7275d0dc42b49d2314a0020ef2d134463ae1e6c0b2c54a103`。只下载并运行新工具，没有安装该提交重新构建的同版本 MSI。

21:18 完成目标机结果核对：三个连续 Session/CurrentUser 创建—读取—停止—移除循环均通过、最终退出 0。每轮实际成功提交 4 帧，读取并逐像素确认 3 种不同合成灰阶 NV12 画面；停止生产者后 1 秒内读到中性画面。三次预览 Shutdown、Session 清理和按自身 exact symbolic link 检查移除均返回 0。末次只读快照中听弦设备数/测试进程数均为 0，已安装 DLL 摘要仍匹配；基础听弦进程及启动时间未变，360 驱动/托盘进程继续运行，未修改防护。

本轮通过的是已安装自有媒体源经实际 Windows 管道的合成像素与三次生命周期。未据 720p30 格式声明宣称有效 30 FPS，也未把虚拟输出中性化当成“真实采集 1 秒内释放”的证据，因为物理采集没有启用。

## 本地证据与未测

源码外 D 盘目录 `artifacts/2026-11-06/video-local-install-2031/`、`video-device-<commit>/`、`video-installer-<commit>/`、`video-local-upgrade-<commit>/` 保留安装日志、每轮 stdout/stderr/退出记录、独立摘要及只读状态。没有覆盖旧日志、发布资产或用户资料。

最终证据为 `video-device-c4a8a8e/acceptance.json`、`installed-device.log`、`installed-device-exit.json`；前两次源码缺陷和测试工具清理失败均原样保留。真实眼部 SDK、摄像头、会议软件、性能、主窗接入、基础安装器正常防护下新装仍为后续门槛，不能声明完整眼部功能或稳定版交付。
