# AV-E6c 摄像头原始时间戳与有界输入

日期：2026-11-08。基线 `a6527fc`，独立分支 `feat/windows-eye-output-20261107`。
完整数字人继续冻结；本轮不启动摄像头、麦克风、SDK、播放、虚拟设备或安装器。

## 决策与依据

假设：显式选定的异步 Media Foundation 摄像头输入可以保留设备原始 QPC，并在
固定容量内交付 BGR 帧；停止先撤销交付，随后单独确认回调及设备退役。
决策价值是确定 `QpcStreamFeed` 的原始输入契约，防止把回调延迟误计为采集时间。
时间基准不明、内存越界、旧代次交付、未解决的构建/回归失败均为停止条件。
源码、合成样本、真实硬件、SDK、安装、会议与性能证据分别记录。

微软官方依据（本轮实际核查）：

- [设备时间戳](https://learn.microsoft.com/en-us/windows/win32/medfound/mfsampleextension-devicetimestamp)
  使用与 QPC 同 epoch 的 100 ns 单位；`GetSampleTime` 是相对流时间，不能替代。
- [异步 Source Reader](https://learn.microsoft.com/en-us/windows/win32/medfound/using-the-source-reader-in-asynchronous-mode)
  要求线程安全回调；每次 `ReadSample` 对应一次完成通知。
- [Flush](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-flush)
  异步成功不等于清理完成，须等 `OnFlush`；回调实际释放另行确认。
- [Lock2DSize](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imf2dbuffer2-lock2dsize)
  提供可访问边界、可见顶行与有符号步长；据此验证正/负步长和缓冲大小。
- [高级视频处理](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-source-reader-enable-advanced-video-processing)
  支持颜色转换；本实现先要求原生尺寸、帧率和逐行格式，避免用缩放/补帧伪装合格输入。
- [硬件源属性](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-devsource-attribute-source-type-vidcap-hw-source)
  用于明确当前支持范围，未确认的软摄像头与听弦回采来源一律不准入。

## 实现

`video/windows/input_camera.*` 为可选原生静态组件；构造不枚举、不打开设备。
显式意图和实际设备 symbolic link 必需，全部控制在创建它的 MTA 线程完成。
枚举后精确匹配硬件源；不存在时不选择默认设备。首阶段只接受 stream 0 原生
1280×720、30 fps、逐行输入，并核验转换后的 RGB32 格式。软件摄像头、29.97 fps、
其他尺寸/流号与缺失格式属性当前不支持，不借默认值隐瞒差异。

异步读取由所有者每次最多发起一个请求。回调没有 reader 引用，不形成循环引用，
不递归发起读取；输入槽最多一帧，替换/过期/停止丢弃分别记账。RGB32 仅移除未使用
字节转换为 BGR，不镜像、不重绘。支持有边界的 2D 正/负步长和线性缓冲。
每帧须有原设备 QPC；缺失、未来、重复/倒退、错流、格式变化、权限/设备错误均撤销
待交付帧。视频 gap 保留计数和标志，不复制上一帧；250 ms 年龄在到达和交付处检查。

停止先关闭原始帧入口并清空槽，再执行 Flush、源 Shutdown、释放 reader/回调及
MFShutdown；请求、回调 flush 和最终引用释放分开记录。失败保留原始 HRESULT，
标记 `cleanup_uncertain`，不报告安全重启。每个实例只准一次启动；新实例/代次需要
新的显式操作。驱动调用自身是否能在时间目标内返回仍须真实设备验证。

该组件**尚未接入 Rust 摄像头绑定、QpcStreamFeed 的实际输入和主窗开始按钮**，
也未加入应用安装包。这里的源码实现不能表示完整音视频链路已经可供会议使用。
未来动态库绑定还须确认代码模块寿命覆盖所有异步回调；当前以静态组件承载。

## 验证与失败恢复

本机：Windows 11、RTX 4070 Ti SUPER 的既有环境；本轮只使用 VS 2019/SDK 19041
进行 `/Zs /W4 /WX` 语法检查，不链接/执行新测试 EXE。首轮因缺少 `mfidl.h` 前置
包含和枚举常量有符号转换失败；修正依赖顺序与明确转换，原警告等级不变，最终通过。
本机 360 隔离阻塞仍未解除；不恢复隔离项、不改防护、不搬移/改名执行。

新增原生 CTest 使用实际内存 `IMFSample`、2D buffer 和生产回调，检查：

1. 原 QPC 高于 JavaScript 精度边界也逐位保留，与 sample-relative 时间不同。
2. 正/负步长、padding、通道顺序、短缓冲/越界及缺失时间戳。
3. 单帧容量、替换计数、代次、两处过期检查、gap、重复/倒退/未来时间。
4. 错流、改格式、EOS、权限错误清空已有帧；停后迟到回调不能泄露原画面。
5. 真实回调跨线程、Flush 与实际释放的区分、三次未打开设备的生产 owner 退役。

CI 尚待本轮提交后执行，未提前计作通过。测试代码只调用明确拒绝或参数无效的
`prepare`，不会打开摄像头。CI 下载如需进行，只保存工件作数据并提取文本日志，
不在本机运行测试二进制。

源码外证据：`artifacts/2026-11-08/camera-input-001/`，包含实验声明、每次类型检查
和后续 CI 的源码/结果/日志/摘要。无实际安装包，安装包摘要不适用。

## 未测与下一步

真实相机在颜色转换后是否保留 `DeviceTimestamp`、驱动曝光时间语义、画质/颜色、
处理耗时、设备拔插和权限撤销、实际 Shutdown 时长均待测。没有属性时明确失败，
不以本机到达时钟补造。iPad/Broadcast 等软件输入须独立明确来源政策后才能支持。
后续先接原生绑定及现有媒体 owner/眼部输入生命周期，再接主窗显式开始与权限状态；
实际设备、普通安装、防护兼容、虚拟端点、会议接收端 P95 和长测仍未验收。
