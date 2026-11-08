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

待独立 CI 证明：实际四个 DLL 文件的固定摘要和锁、目录/格式/缺件/篡改拒绝、主窗
IPC 权限与只读状态、真实 CPU 安装后 12 个缺件/篡改拒绝以及四次恢复。
安装测试只使用 runner 新建的合成目录，原件和变更副本保留，不拿现有资料做破坏测试。

## 尚未完成

真实 eye worker/SDK 的发布清单与许可、主窗准备/启动、虚拟设备输出连接、设备选择、
外部排空/重启、正常防护安装、实际输入效果与会议同步仍待完成。本轮没有打开控件或
把完整性称为完整链路成功；旧数字人继续冻结，教育/旧眼部分支不变。
