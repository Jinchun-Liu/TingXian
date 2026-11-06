# 听弦可选视频组件（实验版）

这是独立于普通用户基础安装包的媒体源组件，版本 `0.1.0`。它只部署原生 DLL、只读诊断、摘要清单和许可证，并在 HKLM 64 位视图注册听弦自己的 COM 类。没有眼部 SDK/模型、CUDA、音频、会话数据库或完整听弦程序；安装成功不表示眼部功能已完成。

媒体源未来由 Windows FrameServer 读取，因此使用受保护的 `%ProgramFiles%\TingXian\VideoSource`，需要系统管理员授权。这个目录与 `D:\Applications\TingXian` 的基础应用分开，不迁移基础应用或用户资料。下载、测试产物和日志继续默认存 D 盘。普通用户不安装 WiX、Visual Studio、Python 或 Rust。

安装不创建虚拟摄像头、启动程序/服务、开启采集、调用 GPU 或改动防护。后续用户显式开启输出时才使用 CurrentUser/Session 设备生命周期；当前主应用尚未接入。媒体源 DLL 不含物理摄像头包装器。当前 MSI 未签名，保持实验组件定位，不覆盖任何已有发布资产。

## 安装、停止与恢复

正式尝试前，核对对应 commit、MSI 大小和独立发布的 SHA-256；保留原包作为恢复来源。检查目标目录及其父目录没有重定向或普通用户写权限。第一次安装若发现同 CLSID 的既有注册，安装会失败并保留该注册；先确认来源，不自动删改或添加信任。

只安装经过本轮验收的 `TingXian-VideoSource-0.1.0-experimental-x64.msi`；不要运行名称含 `ROLLBACK-FIXTURE` 的包。系统授权由用户本人完成。可使用系统安装界面，或通过 `msiexec /i <MSI> /norestart /l*v <D盘日志>` 留存日志。安装器面向 x64、build 22000 及以上；CI Server 构建符合数值条件只用于部署检查，不构成 Windows 11 实机验收。

安装后 `tingxian-video-device-probe.exe --probe` 只读检查导出与注册；`machine_registration_present=true` 不代表真实设备、权限、FrameServer 或会议成功。源码测试工具 `tingxian-video-registration-test.exe --registered-no-device` 验证真实 COM 类加载，也不创建摄像头。实际安装版验收记录须明确二者区别。

关闭视频并完成设备 Session Shutdown 后，从 Windows 已安装应用中卸载“听弦视频组件（实验版）”。卸载仅移除 MSI 拥有的文件及注册值，不递归清目录、不清理听弦资料，也不强删未知注册值。恢复组件可重新安装保留且校验过的 MSI；用户资料无需回滚。若 DLL 被占用、写入失败或系统要求重启，保留错误码和日志，不把中断认作成功，不强行结束会议软件或防护进程。

同一产品版本只对应一次组件交付；后续正式组件更新须同时递增 MSI 三段版本并更换 Product Id，保持 UpgradeCode 与文件组件标识。当前没有验证从旧组件版本升级，只有新装/卸载/同包重装的检查计划。

## 开发构建

Visual Studio 2022 x64、Windows SDK ≥22000、CMake 与 **WiX 3.14.1**。该 WiX 版本为 GitHub Windows runner 已有工具；构建脚本核对版本，不下载或改动本机全局工具链。

```powershell
cmake -S video/windows -B D:/TingXian-build/video-component -A x64
cmake --build D:/TingXian-build/video-component --config Release --parallel 2
./packaging/windows-video/build.ps1 `
  -BinaryDirectory D:/TingXian-build/video-component/Release `
  -OutputDirectory D:/TingXian-build/video-component-msi-round-1
```

输出目录必须不存在；脚本从干净、已提交源码构建清单，记录 commit、逐文件摘要、工具版本与 MSI 摘要。生产 MSI 使用标准文件/注册/回滚操作，没有可执行自定义动作。构建产物未自动安装。

## 隔离 CI 验证

假设：受保护的系统组件可正确注册并回收自己拥有的资源；冲突、写入失败和事务中断不会留下假成功或覆盖已有内容。决策价值：是否可交给用户做正常防护下的独立组件安装验证。停止条件：摘要或 ACL 不符、错误退出码、COM 激活失败、回滚残留，或未知文件/注册值被删除；失败须定位，不能放宽检查。

独立工作流 `Windows optional video component installer` 在 Windows Server 2025 的一次性 GitHub runner 上构建并检查：

1. 已有合成注册的拒绝与保留。
2. 用目录占据 DLL 文件路径，检查非零失败和资源回滚；夹具移到证据目录。
3. 独立回滚夹具在文件/注册脚本实际执行后触发 MSI 内置错误，检查回滚。
4. 两轮安装/卸载，逐项核对摘要、基本 ACL、只读诊断、真实 COM 激活和已安装 DLL 的原生样本测试。
5. 组件目录中的未知合成资料与未知注册值在卸载和重装后仍保留。

`test-install.ps1` 拒绝在普通机器或非 GitHub 托管环境运行。它不打开真实资料、音视频或会议设备。包的 COM 注册与实际虚拟摄像头创建分别验收；CI 不替代目标机正常防护安装、UAC、服务身份权限和会议测试。

参考：[微软组件部署示例](https://github.com/microsoft/Windows-Camera/blob/626f8b19c5f367602f2e89c6b314573d3776c9df/Samples/VirtualCamera/README.md)、[WiX package scope](https://docs.firegiant.com/wix3/xsd/wix/package/)、[WiX upgrade scheduling](https://docs.firegiant.com/wix3/xsd/wix/majorupgrade/)。
