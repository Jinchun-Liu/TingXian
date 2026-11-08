# AV-E7a 自有媒体组件随包与完整性检查

2026-11-08；基线 `e0ee95c`；独立分支 `feat/windows-eye-output-20261107`。

## 决策与范围

假设：普通 CPU 安装包可携带四个自有原生媒体适配 DLL，并由应用内固定的清单摘要
验证；缺件、篡改文件、同时替换可写清单均不能变成可用的启动依赖。决定下一步主窗
启动能否从已知安装位置构造可信原生配置，不接受 WebView 提供的代码路径/摘要。
错误接受、非预期第三方内容、实际采集/播放/SDK/注册、防护改动或不可解释的失败均
为停止条件。完整性不是驱动安装、模型就绪、ABI 成功加载或会议同步证据。

## 实现

- 既有 staging 构建检查 worker，同时构建 `TingXianCameraInput.dll`、
  `TingXianAudioCapture.dll`、`TingXianAudioOutput.dll`、`TingXianVideoOutput.dll`。
  全部使用现有静态 MSVC runtime，放入 `video-worker/media/`，沿用 CPU 安装包资源
  映射；没有 NVIDIA、模型、音频驱动或虚拟摄像头 COM 注册。
- 清单固定格式/顺序/四种角色，不能指定其他路径；构建校验 x64 DLL 头、每文件摘要
  和目录成员，再把整个清单摘要编入主应用。旁边的文本清单不是新的信任来源。
- 安装运行时沿用拒绝重解析点/硬链接的文件验证，保留父目录和文件锁，从同一已验证
  句柄读取清单/PE 元数据。`Bundle` 可生成原生输入/输出 pins；以后加载仍须复查 ABI。
- `video_components` 只允许主窗读取报告，没有路径或摘要参数，不加载 DLL、不枚举
  设备、不启动进程或媒体。安装 CLI `--video-components-check` 在应用/资料初始化前
  使用同一校验逻辑。`integrity_verified` 仅代表四个适配 DLL 完整；UI 尚未接入此接口。
- 缺失开发 bundle 明确 `not_bundled`，不影响旧资料/转写。release 构建必须先 stage；
  这不要求普通安装用户具备任何开发工具。

## 验证与证据

本机 Rust `cargo check --tests --locked --offline` 通过；PowerShell 仅语法解析。
未链接/执行本机测试 EXE，既有 360 拦截保持阻塞。无采集、播放、SDK 执行、安装、
设备注册或防护改动。源码外证据：`artifacts/2026-11-08/media-bundle-001/`。

源码 `bd2c443daec6bceabb0de4127ad404d875377d24` 的
[原生 CI 42459004622](https://github.com/Jinchun-Liu/TingXian/actions/runs/42459004622)
完整日志确认 **154 Rust、11+2+5 CTest、1696 个源码文件检查通过**，没有 ignored。
新增 7 项覆盖固定角色/路径/格式、实际四个生产 DLL 的摘要/文件锁、缺件/硬链接、
DLL 与可写清单同时篡改、未审阅目录成员和未固定组件默认不可用。未加载这些 DLL。
平台工件元数据：5154196 bytes，SHA-256
`1aa613b74acb2b41596bc63275aff8efa67b0454cb7b4904a95b6ec6e5bfb6af`；没有下载/执行
该原生工件，不能称本机摘要复核或本机运行通过。

[Windows 应用 CI 42459005326](https://github.com/Jinchun-Liu/TingXian/actions/runs/42459005326)
同一源码完整成功。前端回归及 TypeScript、50 项核心测试、2 项实际 Tauri MockRuntime
IPC/退出测试通过；新增命令走实际 invoke，展示窗被拒绝，主窗读取不改变眼部代次。
CPU 安装器构建成功，在 runner 的新中文路径完整安装；876 个原离线运行时文件摘要
匹配。安装版主程序实际返回 17 份媒体报告：初始完整 1、四个 DLL 各自的缺件/
文件篡改/同时改写旁边摘要清单共 12 次拒绝，以及四次恢复后成功。
这些检查均未加载四个媒体适配 DLL、未启动采集或输出；不能视为 ABI 或设备验收。
必需 DLL 写入失败仍退出 2；离线自检、CI 平台检查和卸载保留合成用户资料通过。
安装测试只使用 runner 新建的合成目录，原件和变更副本保留，不拿现有资料做破坏测试。

文本证据工件 `13609451468` 已下载到 D 盘，实测 **80257 bytes**，SHA-256
`dfce945517dcd13b78560f110783a09ce59d4ad6bc31e34e75811fdb0c223af3`，与平台一致。
仅提取 stdout/stderr、IPC 测试与回执文本，17 份报告逐一解析核对；没有提取/执行二进制。
完整安装器工件 ZIP `13608883187` 仅核对平台元数据：735106653 bytes，SHA-256
`0cffe9c7561baa8e082a40e870e546d7fd5c09a93261a46fb03ccdd15f2d043c`。
这是 ZIP 的平台摘要，不是安装 EXE 的本机实测摘要；本轮未下载/安装该 EXE。
其内 `SHA256SUMS.txt` 留待实际下载安装时核对，公开历史 release 没有替换或升级为稳定版。

传输异常单独保留：GitHub 插件两次更新 ref 遇到连接故障，现场核对远端未变。
提前派发到旧提交的 run `42458947430` 已取消并确认终态，不计入验收。
随后通过已有 `gh` 登录、重新核对旧 HEAD、`force=false` 更新到原始提交，核对
正确源码再触发 Windows CI；没有改写提交、读取凭证或重复派发同一待观察任务。

## 尚未完成

真实 eye worker/SDK 的发布清单与许可、主窗准备/启动、虚拟设备输出连接、设备选择、
外部排空/重启、正常防护安装、实际输入效果与会议同步仍待完成。本轮没有打开控件或
把完整性称为完整链路成功；旧数字人继续冻结，教育/旧眼部分支不变。
