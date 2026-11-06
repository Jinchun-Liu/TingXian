# NVIDIA AR SDK 0.8.7 Ada 独立路线

2026-11-06—07，`feat/windows-eye-local-20261106`。按用户指令优先核对官方旧版运行时；新版 NGC 权限不是这条路线的前置条件。完整数字人继续冻结，脸部只保留禁用入口。

## 来源与版本

- [官方资源页](https://www.nvidia.com/en-us/geforce/broadcasting/broadcast-sdk/resources/)分别提供 GPU 代际运行时，RTX 40 使用 Ada。
- [官方 Ada 安装器](https://international.download.nvidia.com/Windows/broadcast/sdk/AR/nvidia_ar_sdk_installer_v0.8.7_ada.exe)：实际下载 1,330,863,768 bytes，SHA-256 `0e2e1110065ed58b9c4a844c674fe5f50ed5fe37a0d170bd229a4bbc48861ca5`。这是本次取得文件的摘要，并非 NVIDIA 另行公布的摘要。
- [官方源码](https://github.com/NVIDIA-Maxine/Maxine-AR-SDK/tree/f54db7ecaa0534b696d6194ac7252fc39e4d94a1)固定 `f54db7ecaa0534b696d6194ac7252fc39e4d94a1`，`version.h` 为 0.8.7.0，changelog 对应 0.8.7 运行时。不要改用新版 1.x 示例 ABI。
- [系统指南](https://docs.nvidia.com/deeplearning/maxine/ar-sdk-system-guide/index.html)说明 GitHub 头文件/代理与运行时可组合开发，依赖随运行时提供；官方安装位置为 `C:\Program Files\NVIDIA Corporation\NVIDIA AR SDK`。本轮安装界面未提供目录选择。下载、构建、素材与证据在 D 盘，不另装全局 CUDA 或替换驱动。
- 该包包含 `nvARPose.dll`、`NVCVImage.dll`、CUDA/TensorRT 依赖及 Ada `_89` 模型。不得把 installer/helper 的 x86 PE 当成 x64 推理依赖。

## 许可与来源边界

实际包内 `NVIDIA Maxine EULA.pdf` 为 16 页，SHA-256
`a22811660592e3efec09f135f54e56c50f4470a2414cf8383730ab2bee2efd29`，包含 2024-06-05
Software License Agreement 与 2024-06-13 AI Product-Specific Terms。不是网上仍可找到的
2021 年六页 Maxine SDK 协议，也不能用新版 NGC 页面的许可概述代替包内原文。

| 用途 | 当前依据与边界 |
| --- | --- |
| 本机使用、开发测试 | 基础协议 1.1 与附加条款 2 提供安装/使用等授予。基础 17.4 将公开免费软件定义为 Community Products；这一公开下载方式符合该定义的描述，但不是另行取得的 NVIDIA 资格认定。Enterprise 的订阅/开发计划限制须按对应交付类别判断，不能只因同一 PDF 含企业条款就要求先购买。 |
| 示例源码 | 固定仓库的 MIT 许可仅覆盖对应源文件和通知，运行库与模型不因源码 MIT 而变成 MIT。 |
| 商业使用 | 尚未作生产用途批准；具体产品类别、适用条款、第三方依赖、用户约定及品牌要求须按最终交付核对。没有购买、订阅或续费。 |
| 再分发 | 附加条款含有条件的 Compatible Application 分发授予，基础 8.5/8.8 仍有限制。尚未批准把 DLL、模型或安装器纳入听弦公开仓库/安装包。 |
| 测试数据公开 | 基础 8.9 对 benchmark、竞争分析、regression/performance data 的对外披露要求 NVIDIA 事先书面许可。本轮详细 SDK 运行/效果证据保留在本地；本仓库的合成校验器测试不加载 NVIDIA 软件。 |

Windows 对该安装器及核心 DLL 的 Authenticode 信任检查返回 `0x800B010A`，无法建立
`NVIDIA DVS Dev Signing` 到受信任根的证书链；不能记为签名验证通过。安装器的签名数学
验证及 PE 内容摘要匹配，带 2025-01-21 时间戳，官方 HTTPS 来源保留。这些完整性证据
不等于操作系统对发布者的信任。没有导入根证书、关闭防护或添加排除项；生产交付仍须
决定如何处理这一来源信任缺口。

## 两种清单结构

`scripts/verify-eye-runtime.py` 的 schema 1 保留新版 NGC Core/三个特征包结构。
schema 2 是独立配置，增加 `distribution: "ar_sdk_0_8_7_ada"`，仅接受一个同名 package，
精确版本 `0.8.7.0` 和上述 Ada 官方 URL。禁止混入 NGC 包、换成 Ampere、自动 latest 或
带凭证/query 的下载地址。其余字段和独立审阅清单摘要要求不变。

旧版配置要求核心 DLL、随包依赖、脸框/关键点/眼部编码器与解码器模型、实际 EULA 及
第三方通知。清单面向经审阅的 x64 执行文件集合，不直接将整个安装目录的卸载/安装
辅助程序纳入。没有放宽重定向/硬链接、PE 架构、缺件、篡改或文件角色检查。
验证结果仍只称 `integrity_verified`，许可、签名信任、GPU 加载和效果另验。

假设：独立的旧版配置可接受完整合成 x64 树，并在执行前拒绝模型/依赖缺失、错误包型
或平台。它决定后续组件清单格式；任何错误接受都停止采纳。27 项本机合成检查通过，
含原有 20 项。证据在源码外 `artifacts/2026-11-07/eye-legacy-manifest-001/`；全部夹具保留，
没有用这些结果声明 NVIDIA 功能或许可通过。

源码 `ffaaf760227def8d15879a5c60c51b906d8a1d60` 的[独立完整性 CI](https://github.com/Jinchun-Liu/TingXian/actions/runs/42252552995)在 Windows 2022 与 Ubuntu 均通过；只生成合成 PE/模型字节并验证本仓库校验器，不下载、执行或评测 NVIDIA SDK。

## 本地验证与下一阶段

实际取得文件、许可全文、环境、安装观察、离线输入/输出、网络隔离和失败恢复记录在
源码外 `artifacts/2026-11-06/eye-ar087-233011/`。不将该目录或媒体上传 GitHub。
使用官方固定源码自行构建，离线命令必须明确 `--offline_mode=true`、独立输入/输出路径，
`--split_screen_view=false` 与 `--draw_visualization=false`。`--cam_res` 不决定离线尺寸。
检查退出码、日志、解码、帧数/时长/音轨与成对画面，程序退出 0 不能单独证明眼部效果。

宿主联网而测试 EXE 的出站访问被独立阻断，须称“进程网络隔离”，不冒称整机物理断网。
网络素材须保存原来源/许可；处理副本无音频，原件保留。没有启动物理摄像头、麦克风或
播放声音，没有把本地预览当作飞书输出。

后续按本地证据决定是否接入独立 `EyeCorrectionProvider`。禁止直接把示例的文件循环
当成会议后端：仍需显式初始化/停止、真实错误传播、置信度及丢脸状态、有限队列、旧帧
隔离、默认中性输出与重新授权。运行时缺失不影响基础听弦，展示窗不获得处理权限。
目前听弦本身仍只有无媒体检查接口；未把第三方示例运行写成应用集成或稳定版交付。
