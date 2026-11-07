# AV-E1 原生时间线：第一部分验收

范围基线：[AV-SYNC-20261107](eye-av-sync-scope-2026-11-07.md)。开发分支
`feat/windows-eye-output-20261107`，起点 `0a8ddaf440b1aa3bc5c0ee75bca1acf74b25726d`。
本轮新增 `crates/video-worker-control/src/av_timeline/`；未改现有视频 v1 协议、
主窗命令、安装包或教育代码。完整数字人继续冻结。

## 决策与边界

假设：独立设备时钟与原始 PCM 的采集时间可映射到有界的同步窗口，持续漂移不会
变成不断累积的样本偏移，停止后不交付旧代次窗口。
决策价值：确定后续原生重采样器、WASAPI 适配器和视频 v2 协议的时间/采样位置契约。
停止条件：错源、单位混淆、旧数据交付、无界积压或未记账丢失先修复；不重复相同失败。
影响产物：时间线核心与本记录。模拟时间及窗口交付不证明真实设备、声音质量或会议同步。

## 源码行为

- 原生接口默认关闭，无媒体、网络、ASR、Tauri 命令或系统设备副作用。
- `Time` 显式使用 100 ns QPC；原始 ticks 按频率换算。输出设备位置必须携带
  `GetFrequency` 单位，两次有效观测后才提供预测区间。相同位置可重复观测，但不延长
  有效时钟租期；失效后需重新建立设备时钟。调用者遇到时钟错误须同时撤销两路端点。
- 输出设备时钟及生成的区间绑定 generation，不能给旧区间换上新代次后复用。
  采集序号连续、采集时间单调、格式一致才接收 PCM；明确拒绝系统回放、输出回流和
  非选定设备。真实端点身份仍须由原生采集适配器核实，枚举值本身不是设备鉴权。
- 原始样本原样保留；设备标记为静音时按零样本接收。返回整数样本位置及小数偏移，
  供后续合适的重采样滤波器使用。整数分数运算避免恰好落在采样点时退到前一采样。
- 依据采集时间排列已校正 NV12 帧；迟到/过期/超上限视频可丢弃并计数。PCM 超上限
  则清空并故障，不用最新块覆盖连续语音。缺少任一路有效数据时回调 `None`，约定
  两路中性画面/静音，并推进截止位置，后到数据不能补播已经错过的区间。
- 借用窗口交付与停止共享锁，回调必须短且不阻塞、不可重入；拒收或 panic 使两路
  时间线清空并故障。重启需显式 stop/begin。真实设备缓冲的撤销和看门狗仍是端点层职责。

本轮参数仅供无设备实验：48 kHz 单声道 f32、单包 ≤960 帧、PCM 缓存 ≤9600 帧
（200 ms）、720p NV12 视频 ≤8 帧、视频年龄 ≤250 ms；目标缓冲 40 ms、源帧落后
≤50 ms。设备观测间隔/预测上限 100 ms，速率边界 ±2000 ppm，考虑一 tick 时间戳量化。
尚未根据真实眼部耗时或音频驱动确定产品默认值，超边界采取故障/中性输出而非无限等待。

时间基准依据：[GetBuffer 首样本位置与 QPC](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)、
[GetPosition 位置单位、停启与重复观测](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclock-getposition)。
核心尚未调用这两个系统接口；本轮用独立合成振荡器验证映射。

## 本机结果

Windows 11 Pro 10.0.26200，Ryzen 7 9700X，RTX 4070 Ti SUPER/32.0.16.1088。
Rust 1.99.1、MSVC 19.29；工具链与构建目录在 D 盘。未加载 NVIDIA SDK/模型，
未更新安装器或运行中的应用。GPU 在本轮仅为环境记录，不是测试执行器。

命令：在配置 MSVC、D 盘 CARGO_HOME/RUSTUP_HOME/CARGO_TARGET_DIR 后执行
`cargo +1.99.1 test -p video-worker-control --locked --offline av_timeline:: -- --test-threads=1 --nocapture`。

| 检查 | 实际结果 | 证据边界 |
| --- | --- | --- |
| 首轮 12 项 | 11 通过、1 失败：精确采样点 603 被浮点 floor 算成 602 | 保留 `timeline-first.log`；未削弱原断言 |
| 修复与定向补充后 | 17/17 通过，42 项既有测试被过滤，不计为本机通过 | `timeline-final.log`；包含跨包精确样本、量化、静止时钟、单路欠载、跨代次时钟 |
| 两个独立漂移时钟 | 模拟 600 秒，59,996 个就绪窗口；源位置误差 <0.02 样本，日志六位小数为 0.000000 | 非 600 秒墙钟运行；没有真正重采样、播放或测接收端 |
| 有界性 | 该漂移案例 PCM 峰值 2,880 帧、视频峰值 3 帧 | 另有硬上限故障测试；不是 CPU/内存/真实延迟性能结果 |
| 停止和恢复 | 三次重启清空两路；旧音频/窗口/时钟隔离；拒收及异常清空 | 回调返回代表交付，不代表真实设备消费；指标不冒称音频已到会议 |

同一段合成 PCM 的样本值、序号与视频采集标记均核对；本轮没有读取真实会话或资料。
指标区分入队、退休、清空、静音、窗口交付、唯一/重复/丢弃视频；退休样本数不是送达数。

功能提交 `034b8a98e2c8018c6e5aecd15c256c9454488b1c` 已推送；同源码
[Windows 原生 CI 42373277945](https://github.com/Jinchun-Liu/TingXian/actions/runs/42373277945)
通过完整 59 项 Rust（0 过滤）及 7+2 组 CTest，公开源码检查通过。完整日志已保存在
本机 `ci-job-128405604613.log`，其中包含对应 checkout SHA；未下载/执行新 CI 二进制。
工件 ID `13574338935`、2,950,484 bytes，GitHub 报告 SHA-256
`05d03bc925aef1d30be7b13c2778f35ab66198a1016685a7d09a7162fa8bd672`，仅核对元数据。
这不是安装器构建、目标机设备加载、NVIDIA SDK 或飞书验收。

此前教育范围文档 `ca60ba8` 也已正常推送；GitHub 分支集合读取确认教育、眼部输出、
眼部基线和数字人归档四个 HEAD。一次 git ls-remote 遇 schannel TLS 失败，另一次
GitHub 工具拒绝带编码斜线的单分支 URL；改用已登录连接的分支集合只读接口完成核对，
没有修改 TLS、防护或凭证。

首次格式化命令未继承 D 盘工具链变量，触发默认目录工具链补下载并 TLS 失败，
未启动测试；保留诊断，不删除缓存。随后明确设置 D 盘环境，只补装 rustfmt 后格式化。
已有工作树无 reset/clean/强推；Mac、教育和冻结归档分支均未整合到本实现。

## 音频端点候选：只下载、只核查

2026-11-07 UTC 12:31:42 从 [VB-CABLE 官方下载](https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip)
取得 Pack46，1,324,680 bytes，SHA-256
`bc44e8974191a5a2427d6c979e2dab08dbb09925f3f435dfa2f0ba3e45b6eb90`。
31 项解包清单保存在源码外，未执行任何安装程序。

| 核查项 | 实际结果 |
| --- | --- |
| x64 setup | 2.1.5.8、937728 bytes；SHA-256 `734c35dfa6d98f48782a451633ceb471166ec70d60482fd89a1123d0ee3c4f41`；Authenticode Valid，发布者 BUREL VINCENT Entrepreneur individuel |
| x64 驱动 | 3.5.2.0，INF 日期 2026-10-31；驱动和目录文件 Authenticode Valid |
| 内核目录成员校验 | `signtool verify /kp /v /c vbaudio_cable64_win10.cat vbaudio_cable64_win10.sys` 成功，0 警告/错误；目录签发给 Microsoft Windows Hardware Compatibility Publisher |
| 安装/端点/消费 | 未执行，签名成功不代表本机加载或会议可用 |

首次 PowerShell 下载遇 TLS EOF，记录的 0 bytes/null hash 不代表成功；改用正常证书
验证的 curl 从相同官方 HTTPS 地址取得包。未降低 TLS 验证或防护。首次解包因调用了
错误的 .NET 重载而失败，未解出文件；改用 ZipFileExtensions，仍先校验路径和大小。

[产品页](https://vb-audio.com/Cable/)说明 CABLE Input 为播放端、CABLE Output 为录制端，
需管理员安装并重启。[许可页](https://vb-audio.com/Services/licensing.htm)允许先测试；
对原包与其他产品一同分发列出保留来源、Donationware 标识及专业组织许可等条件。
实际包内 `readme.txt` 允许个人/单一实体使用与原样扩散，但将并入其他安装程序要求
作者同意；两处表述不完全一致。因此当前决策是**独立评估候选，不并包、不改名、
不购买、不将其称作已交付的听弦虚拟麦克风**。商业/组织使用和产品集成许可仍待核实。

备选 [Microsoft SysVAD](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/)
是驱动开发样例，需 WDK/SDK 等；不是现成已签名的产品音频桥。
[驱动签名要求](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-reqs)
还涉及 Hardware Dashboard/EV 资格。现阶段不购买证书、不启用测试签名、不修改安全启动，
也不把这一成本作为推进纯原生适配器工作的前置条件。

## 剩余出口

1. AV-E1 仍须完成实际重采样/滤波延迟与有界调度适配，不把源位置映射当成音频输出。
2. AV-E2 核对外部桥的输入/录制端配对、端点身份、停止后残余缓冲和正常防护下安装。
   尚未安装新音频设备，未改变默认设备；相应系统授权和重启需保护当前工作后单独处理。
3. 视频 v2 需显式传递 QPC/代次/呈现时间，接通共同排期；不能把 v1 的毫秒时间直接改名。
4. 原生所有者、真实采集、主窗状态、可选组件打包、目标机新安装版仍待实现/验收。
5. 真实人物、飞书接收端 P95 ≤100 ms、30/60 分钟稳定性和商业分发许可分别待验证。

本机完整证据目录：源码外 `artifacts/2026-11-07/av-timeline-001/`。仅公开自有源码、
合成回归和脱敏记录，不上传第三方二进制/模型、私人音视频或用户数据库。
