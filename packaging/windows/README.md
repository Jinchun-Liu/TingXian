# 听弦 Windows 测试版

目标系统为 Windows 11 x64。0.7.0-beta.1 使用 CPU Whisper，普通问答不要求 NVIDIA 显卡。CUDA 和数字人暂未交付。真实会议、蓝牙与 USB 设备、无高端独显机器仍须按 `docs/windows-validation.md` 验收。

## 安装

下载发布页的 `TingXian-0.7.0-beta.1-Windows-x64-setup.exe` 和 `SHA256SUMS.txt`。使用 `Get-FileHash <安装包路径> -Algorithm SHA256` 核对，然后双击安装器。安装到当前用户，不要求安装 Node、Python、Rust 或 Visual Studio。安装器附带 WebView2、Whisper small、Python 文档解析和 FFmpeg。此测试包未作开发者证书签名。

从开始菜单打开「听弦」。音频设置选择会议正在使用的输出设备，按需开启「记录我的声音」。系统输出通过 WASAPI loopback 采集，麦克风独立作为本人上下文。首次本地转写不需要 Key。需要 AI 时再配置自己的接口并开启发送许可。

### 本机无声安装检查

静默安装退出码为 0 仍可能留下不完整资源；必须验证安装内容。持有源码时可用 PowerShell 运行以下检查，无需 Node、Rust、Visual Studio 或系统 Python：

```powershell
./scripts/check-windows-local.ps1 -InstallDirectory "$env:LOCALAPPDATA\听弦" -EvidenceDirectory ./local-evidence-round-1
```

证据目录必须尚不存在。脚本核对全部随包清单、独立打包的 ONNX 文件是否存在，再以移除开发工具的 PATH 执行内置离线自检；缺失、摘要错误和退出码写入 `result.json`。它不采集音频、不使用模型接口、不打开个人会话数据库。可选 `-PlatformCheck` 额外执行内存截图和合成可恢复回收检查。

若安装器提示 DLL 无法写入，停止安装并保留错误路径，不选择忽略后继续使用。不要用系统中的同名 DLL 拼补安装目录。先排查安装目录、运行环境和具体错误，再重新安装并重跑检查。实机记录见 [Windows 首轮接手验收](../../docs/windows-local-validation-2026-11-05.md)。

后续源码构建通过 `installer-hooks.nsh` 关闭 NSIS 的跳过文件行为：必要文件写入失败时，交互安装只提供重试/取消，静默安装必须返回非零。CI 用合成目录占据 DLL 路径验证此失败分支，再验收正常安装；该保护尚不改变原 beta.1 发布包，也不解除系统拒绝写入的原因。

从桌面代理的打包进程直接启动应用时，用户目录可能被重定向到代理自身的缓存。首次模型准备若出现 `os error 17`，先从 Windows 文件资源管理器正常启动并对照；不要移动用户资料、关闭防护或修改模型复制逻辑来掩盖启动环境差异。本机已复现“代理子进程失败、Explorer 启动正常”；安装器 DLL 写入失败仍是另一个未解决的问题。

同一重定向也会影响命令行读取与备份：即使显式写出 C/D 盘的用户目录，也可能读到代理的 `LocalCache/Roaming`。备份前核对已打开文件的最终路径，并与桌面应用的索引数量、资料根目录和偏好对照。路径或内容不一致时停止清理/升级；保留该副本并标明范围，通过普通桌面资源管理器取得实际资料副本后再验证。空数据库、复制成功和摘要一致都不能单独证明备份了正在使用的资料。

Windows 麦克风权限在「设置 → 隐私和安全性 → 麦克风」中管理。设备断开或同名无法区分时重新选择或在系统设置重命名，不会默默改录其他设备。GPU 加速并未包含在这个 CPU 测试包内。

## 升级、卸载和回滚

升级前停止采集并保存会话。运行新版安装器覆盖当前安装。应用标识沿用 `io.github.jinchunliu.meetilyinterview`，用户资料位于 `%APPDATA%` 下的同名目录，文件路径以应用实际显示为准。

「设置 → 应用 → 已安装的应用 → 听弦」可卸载程序。卸载不清除用户资料。要清理会话或索引，使用应用内独立入口，它先创建可恢复归档，再清除数据库记录。回收失败时停止清理。

回滚前备份上述用户目录。保存此前的安装包和校验文件。先卸载应用，再安装旧版，仅在旧版明确兼容当前数据库时直接读取，否则恢复对应版本备份。不要用旧库覆盖较新的个人记录。

## 开发与构建

开发者需要 Windows 11 x64、Visual Studio 2022 C++ Build Tools 和 Windows SDK、Rust 1.99.1 MSVC、Node 22、pnpm 9.15.9、Python 3.12、LLVM/libclang 与 CMake。这是开发要求，普通安装者不需要这些工具。

先打开 Visual Studio 2022 的 **Developer PowerShell**（x64 环境）。`VCToolsRedistDir` 必须指向已安装的 C++ 运行库目录，脚本据此打包普通用户所需的 DLL。

```powershell
git clone https://github.com/Jinchun-Liu/TingXian.git
cd TingXian
git switch feat/windows-eye-local-20261106
corepack enable
corepack prepare pnpm@9.15.9 --activate
python scripts/prepare-windows-runtime.py
./scripts/stage-windows-video-worker.ps1
cd frontend
pnpm install --frozen-lockfile
pnpm exec tauri dev
```

发布构建流程见 `.github/workflows/windows-preview.yml`。所有下载版本与摘要见 `runtime-lock.json`，依赖许可见 `THIRD-PARTY.md`。CI 的离线资源/安装检查通过并不等于真实设备音频、截图或 60 分钟会议已通过。

当前开发分支另随包编译本地视频检查 worker，摘要在编译主程序时嵌入。主窗显式检查时才运行；退出会停止并回收，缺少已批准眼部运行时返回不可用。它不启动摄像头、音频或 SDK，也不注册虚拟设备。`meetily.exe --video-worker-check` 用同一控制路径作无采集安装检查；检查成功不代表眼部处理可用。普通用户仍无需 CMake、Visual Studio 或 CUDA Toolkit。

上面的独立眼部分支沿用 Mac 0.6.5 UI 基准；`feat/windows-prd-v2` 保留原 Windows 交付历史，不是本轮视觉功能源码。不要回退已有后续提交。

主窗视频 IPC/退出回归使用 `scripts/test-windows-video-app.ps1 -EvidenceDirectory <新证据目录>`。
先按上述步骤准备组件和依赖。脚本保留导入表与清单证据，仅给测试程序副本补齐 Common
Controls v6 清单；不会修改生产程序清单、安装系统 DLL、打开设备或跳过失败断言。
