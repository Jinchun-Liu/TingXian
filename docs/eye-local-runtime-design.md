# 本地眼部组件与 Windows 输出决策

核查日期：2026-11-06。适用分支 `feat/windows-eye-local-20261106`，依据 [PRD v2.1](PRD-v2.1-eye-face.md)。Mac 0.6.5 的既有界面、普通 CPU 安装包和用户资料格式不变；完整数字人路线仍冻结。

## 当前决定与缺口

优先评估 NVIDIA Maxine 原生 Windows x64 Eye Contact，作为独立可选组件。暂不把 NVIDIA DLL、模型或安装器纳入仓库或基础包。当前没有取得经过核对的 x64 SDK/模型下载包，未固定其实际版本、摘要、驱动最低要求与账户许可；没有已批准的生产组件清单。

实现离线完整性检查作为组件取得后的第一步。该检查读文件和 PE 头，不加载 DLL、执行下载脚本、注册设备或接受条款。通过结果只能称为 `integrity_verified`；GPU 兼容、模型加载、许可和摄像头仍是独立门槛。尚未接入应用启动或 UI，不会改变当前安装版。

NGC 资料和免费的 NVIDIA Developer Program 登记已完成，实际会员入口可访问，未订阅营销通讯。登记后重新加载 Core 资源页，下载按钮仍禁用，页面要求 AI Enterprise 订阅，版本表被遮挡；没有读取受限版本内容或尝试绕过访问限制。当前可以确认账号登记与下载授权是两个门槛，不能宣称必须购买或已获 SDK 使用权。

[官方 AI for Media 页面](https://developer.nvidia.com/topics/ai/generative-ai/ai-for-media)列有 90 天免费评估申请渠道。续查 NGC 当前组织 Subscriptions 为 0；[评估入口](https://enterpriseproductregistration.nvidia.com/?LicType=EVAL&ProductFamily=NVAIEnterprise)明确要求企业邮箱及兼容基础设施，不批准个人邮箱。用户补充学校邮箱后已进入新账户创建页，尚待用户自行设置凭证及验证邮箱；教育机构邮箱是否获批、该硬件是否适用及生产/分发许可仍未确认。没有把学校邮箱冒称企业资格，公开记录不含邮箱。尚未提交试用/销售申请、支付、生成 API Key 或下载 SDK。后续先确认授权类别，再固定 x64 Core、三个特征、摘要和许可；不以旧包或云端外发替换当前需求。免费计划登记适用的[Technology Access Terms](https://developer.nvidia.com/legal/terms/)与各软件产品协议仍须分开核对。

## 官方组件与版本

| 来源 | 现场结果 | 实施影响 |
| --- | --- | --- |
| [Eye Contact 集合](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/collections/nvargazeredirection/-) | Core 与特征组件分开，功能覆盖视线估计及范围内校正 | 下载清单必须覆盖特征依赖，不能仅检查主 DLL |
| [Core 资源](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/resources/ar_sdk_core/-) | 概览显示 `1.2.0.0_woa`、577.44 MB、2026-09-30 | WoA 属于 ARM 路线，不能把页面 latest 当成当前 x64 组件 |
| [Windows 安装指引](https://docs.nvidia.com/maxine/ar/latest/WindowsARSDK/InstalltheARSDK.html) | Core ZIP 加特征安装脚本；脚本可按 GPU、特征和版本选择 | 取得下载后固定版本；不运行默认全部特征或自动 latest 下载 |
| [官方示例构建依赖](https://github.com/NVIDIA-Maxine/AR-SDK-Samples/blob/7b29f828f3581659ce432f1af5d101373db3594a/apps/GazeRedirectionApp/CMakeLists.txt) | 要求 `nvARGazeRedirection:1.2.0.0`、`nvARLandmarkDetection:1.2.0.0`、`nvARFaceBoxDetection:1.2.0.0` | 这是该示例源码的依赖，不证明这些版本已对本账号提供 x64 包 |
| [Windows 要求](https://docs.nvidia.com/maxine/ar/latest/WindowsARSDK/Windows.html) | x64 支持范围包含 Turing/Ampere/Ada/Blackwell；推理运行依赖随 SDK 提供 | 目标显卡属于候选硬件不等于加载成功；普通用户不安装 CUDA Toolkit/VS |
| [Unicode 路径](https://docs.nvidia.com/maxine/ar/latest/WindowsARSDK/UnicodePathSupport.html) | 当前文档说明 UTF-8 路径支持 | 仍需用最终锁定版本在 D 盘中文路径执行加载验证 |

官方示例仓库 main 当次 HEAD 为 `7b29f828f3581659ce432f1af5d101373db3594a`；Gaze CMake blob `9789609efdde9c0b6672b9fe599b9d95590770c7`。本轮只读源码，不拉取 LFS 视频或运行摄像头脚本。其 C++ 调用可用于设计，但正式 ABI、枚举、加载路径和 GPU 同步策略必须以所获 SDK 头文件与实际运行验证为准。

[眼部 API 文档](https://docs.nvidia.com/maxine/ar/latest/API/Architecture/using-ar-features.html)提供自然移视和头姿/视线范围参数。待适配器证实后再显示对应 UI。单人模式不能把“自动选最大人脸”冒称已识别用户；多人进入画面时的安全行为需要单独设计与验证。

## 许可分层

- [示例代码 MIT](https://github.com/NVIDIA-Maxine/AR-SDK-Samples/blob/7b29f828f3581659ce432f1af5d101373db3594a/LICENSE)仅涵盖该示例仓库。没有将 NVIDIA 运行库、模型或素材改标为 MIT。
- [NVIDIA Software License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-software-license-agreement/)与具体产品条款共同限定使用范围；下载可见不等于可以随产品重新分发。
- [AI Product Specific Terms](https://www.nvidia.com/en-us/agreements/enterprise-software/product-specific-terms-for-ai-products/)在核查时标注 2026-04-15 更新；开发者计划对 Enterprise 产品的内部开发/评估授权，与客户产品分发条件分开。必须确认取得包的实际授权类别，不能直接套用通用分发段落。
- [Community Model License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-community-models-license/)包含模型分发条件及部分本地端侧例外。网页的标题与页尾日期不一致；交付时归档具体包中条款并记录摘要，不能只记录一个可变网页日期。

本轮没有作商业使用或再分发批准。后续分别记录运行库、三类特征模型、样例素材的许可标识、原文摘要、账户授权适用范围和拟用方式。取得评估权后可以进行授权合成输入验证；生产包分发需另行核对，不能由本脚本判定。

## 虚拟摄像头与进程边界

选定 Windows 11 Media Foundation 软件虚拟摄像头作为首个实现候选。基础程序不在启动时注册设备。[MFCreateVirtualCamera](https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-mfcreatevirtualcamera)最低系统版本为 build 22000，支持 CurrentUser 和 Session 生命周期；AllUsers 需要管理员权限。CurrentUser 不意味着媒体源 DLL 可以免系统安装。

[微软官方示例](https://github.com/microsoft/Windows-Camera/blob/626f8b19c5f367602f2e89c6b314573d3776c9df/Samples/VirtualCamera/README.md)通过 MSI 部署媒体源并在 HKLM 注册 COM。媒体源运行在 FrameServer 服务中。因此实现独立、可卸载的输出组件，安装需要系统授权；已在 CI 和目标机部署并激活 COM 媒体源，当前 0.1.2 三轮实际合成设备验证已通过，详见下方分层记录。不能据一般驱动示例断言必须开发内核驱动，也不能承诺所有环境免管理员安装。

主控制窗管理用户意图；独立用户态 worker 管理采集和 GPU，媒体源只消费校正帧与中性占位帧。展示窗不获得帧句柄或启动权限。第一版 IPC 已采用可取消的本机命名管道，核对连接身份、代次和固定载荷，只允许当前登录会话及 LocalService，不开放局域网端口或 Everyone 权限。是否进一步使用共享内存/显存须依据实际帧时间与复制开销决定；当前未做性能达标声明。

原生输出组件从 `6ba5a80` 的进程内媒体源推进到 `7abcfcf` 的双进程帧接收及 MF 激活配置，CI 和目标机各通过 15+10 组，见[IPC 验收](video-ipc-validation-2026-11-06.md)。当前只接受已经处理好的 NV12 字节；主应用/GPU worker 接入与真实 FrameServer 仍待验证，不含眼部推理。

`ddada3c` 新增独立 Session/CurrentUser 生命周期控制层及只读诊断，CI/本机各通过 13 组新增检查。设备 OS 操作为故障注入，生产 API 后端仅完成编译及调用前拒绝检查；目标机确认 API 导出存在而媒体源尚未安装，见[生命周期验收](video-session-validation-2026-11-06.md)。

`9eb55d3` 独立 MSI 完成 CI 与目标机安装后，实际纯合成设备测试暴露激活器析构提前关闭源、格式探测消耗生产者连接。两处已修复，当前目标机安装 `084dae5` / 0.1.2，21 条安装断言通过；`c4a8a8e` 工具完成三轮实际 FrameServer 合成像素、停止中性化及设备移除/再启。16+12+13 组原生及 7+5 组部署/升级 CI 通过。见[设备记录](video-device-validation-2026-11-06.md)与[安装验收](video-installer-validation-2026-11-06.md)。主应用、GPU/SDK、真实摄像头与会议仍未验收完成。

关闭/故障时由输出端自己的看门狗切中性帧，即使 worker 已崩溃也不无限复用最后一帧。恢复只能恢复偏好，不能恢复采集、输出或原画面回退授权。后续实现须验证最多两帧的等待队列、旧 generation 丢弃、GPU 完成计时和独立音频路径；不把写入虚拟设备称为对端已收到。

## 离线组件检查

新增 `scripts/verify-eye-runtime.py`，仅依赖 Python 标准库。开发阶段可以用本机 Python 3.12；未来若调用随包 Python，仍须独立设计入口，当前没有给普通用户新增环境要求。

```powershell
python -I -B scripts/verify-eye-runtime.py `
  --root 'D:\TingXian-components\eye-runtime' `
  --manifest 'D:\TingXian-components\reviewed-eye-manifest.json' `
  --manifest-sha256 '<独立审阅取得的清单 SHA-256>'
```

参数是取得正式包后的用法，不是当前存在的文件或已批准版本。清单放在运行时目录外。不能从待检包读取摘要再当作独立信任依据；本仓库当前没有生产 pin。目录是经审阅的运行时文件集合，不要求把 SDK 开发工具和完整 ZIP 不加筛选地装入应用。

清单 schema 1：

| 字段 | 要求 |
| --- | --- |
| `schema_version` | 整数 1 |
| `provider` / `platform` | `nvidia_maxine_eye_contact` / `windows_x86_64` |
| `packages` | 恰好 Core 与上述三个特征；每项 `id`、精确 `version`、无凭证/query 的官方 NGC `source_url`、`archive_sha256`、`license_id` |
| `files` | 每项 `path`、`package`、`kind`、`bytes`、`sha256`；路径为相对 POSIX 形式 |
| `kind` | `pe` / `model` / `license` / `data`；每个包须有 PE 和许可文件，三个特征还须有模型 |

实现检查清单本身的独立摘要、重复字段、完整组件依赖、文件大小和流式 SHA-256、x64 PE 签名及头部格式。拒绝额外文件、缺件、大小写碰撞、Windows 保留名/ADS/越界路径、符号链接、junction/reparse point 与硬链接。成功返回 JSON 和退出 0；拒绝返回具体 code 与退出 2。日志不输出凭证或绝对用户路径。

这是静态完整性快照，不是 PE 装载器、安全审计或防并发文件替换的沙箱。归档 SHA 在此只记录，下载取得阶段仍须单独核验原 ZIP。清单不能判断文件是否具有真实 NVIDIA 功能或许可是否有效。未来激活必须在受控不可写目录中重新校验并限制 DLL 搜索范围，不能永久缓存本次成功结果来允许任意后续替换。

## 验证决定与停止条件

假设：独立固定的完整 x64 组件树通过，而缺件、篡改、ARM64 二进制、路径重定向会在执行前被拒绝。决策价值是确定该工具能否用于后续组件取得检查；任何错误接受或损坏外部文件都阻止采纳，先修根因。仅使用合成 PE 头、虚构模型字节和无授权效力的许可文本，不加载 DLL、不拍摄、不录音、不发 AI 请求。

`scripts/test-eye-runtime.py --evidence-dir <新目录>` 保留所有合成夹具，不覆盖或删除旧证据。Windows 用实际 junction，Linux 用目录符号链接。CI 的 `Eye runtime integrity contracts` 在 Windows/Ubuntu 运行相同检查，只上传紧凑结果，不上传系统跟踪或用户资料。

待测：实际 x64 包/许可固定；GPU/驱动兼容与离线加载；授权合成输入眼区质量；真实摄像头；虚拟设备安装/停止；音画同步；资源共存；正常防护下基础安装；30/60 分钟会议。以上均未因本轮源码与合成回归而通过。
