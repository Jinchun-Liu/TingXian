# 眼部校正帧到视频输出的连接验收

起点 `b00b63d`；独立分支 `feat/windows-eye-output-20261107`。
教育分支 `4c7b910`、眼部既有分支与数字人归档原位保留，不混入其他设备新增提交。

## 实验决定

- 假设：Provider 的校正后 BGR 可以保留原始帧 ID/时间戳，经有界交接、明确色彩转换和
  现有受限命名管道进入 MF 输出；慢接收端不会阻塞 Provider 的停止锁。
- 决策价值：决定眼部原生输出连接是否可继续进入宿主/预览集成，定位颜色和取消边界。
- 停止条件：原图回退、旧代次恢复、时间戳刷新掩盖过期、队列超界、错误色彩、停止阻塞
  或残留线程。出现时先定位修复，不扩大媒体矩阵。
- 影响产物：原生 OutputSink、媒体格式声明、SDK-free 连接回归及 Windows 验收账本。

只使用自有合成像素与 fake provider，不加载 NVIDIA、不打开物理摄像头、注册虚拟设备、
采集音频、播放声音或访问真实模型。不修改安全软件、既有运行时、安装版和用户资料。
本地证据目录为源码外 `artifacts/2026-11-07/eye-output-001/`。实际结果后续追加。

## 技术依据

使用 720p packed BGR8 到 limited-range BT.709 NV12，2×2 平均色度位于采样块中心。
MF 类型明确声明相同矩阵、范围和色度位置，避免依赖消费者默认值。转换前后必须保留
采集时间；此色彩转换不能作为 SDK 非眼区保真的证据，后者应在转换前的同格式下比较。

依据：[Microsoft RGB/YUV 公式](https://learn.microsoft.com/en-us/windows/win32/medfound/recommended-8-bit-yuv-formats-for-video-rendering)、
[MF 矩阵属性](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-mt-yuv-matrix-attribute)、
[色度位置](https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/ne-mfobjects-mfvideochromasubsampling)。
仅采用格式/数学约定；没有引入新运行时或复制第三方示例文件。

本轮仍不等于摄像头、SDK 到虚拟设备、主应用安装版或会议验收；稳定版门槛保持未完成。

## 本机验证

Windows 11 Pro 10.0.26200，VS 2019 MSVC 19.29，已装 Windows SDK 10.0.19041.0，
CMake 4.4.5。本轮没有调用 GPU 或 NVIDIA SDK。新源码/构建目录与原 worker 分开。

首次构建全部原生目标时，既有设备后端找不到 `mfvirtualcamera.h`；本机尚无 Windows 11
开发头文件。失败保留 `build-native-1.log`。新输出库本身只需要已有管道/Kernel API，已
去掉对设备注册库不必要的链接依赖；没有下载未知头文件或修改系统 SDK。完整原生构建
仍需 CI 的既有 Windows 11 SDK，不能把本机选定目标通过写成全量通过。

本机独立构建媒体源、源测试、IPC 与新增输出库成功，首次针对性 CTest 3/3 通过：
真实 MF 格式/样本、双进程连接、新输出连接。新增延后连接过期案例后，输出目标复测
1/1 通过；不重复无关矩阵。原始日志和后续最终日志分别保留。

- 已知黑/白/红/绿/蓝/灰色 NV12、2×2 混合色度及输入不可变检查通过。
- 默认关闭、旧启动票据/代次/帧序号/时间戳拒绝、最多一个待发结果、三轮明确重启通过。
- 独立原生客户端核对真实管道帧的 generation/id/captured_ms，与输入元数据相同；
  延迟建立连接时过期帧丢弃，不重标时间、不恢复旧画面。
- 慢接收端只读取帧头而不读取大载荷，真实阻塞写入时停止回调仍在 500 ms 内返回，
  关闭和线程回收在 1 秒内完成；不是 GPU 或会议延迟成绩。
- 假 Provider 输入蓝色、输出红色，经两个实际进程进入 MF 样本，核对接收值为红色；
  明确停止、假 Provider 丢脸故障和子进程直接退出均切中性样本，没有原图回退。

源码级新连接尚未接入实际 NVIDIA worker 或 Tauri 控件；本机已装 0.1.2 媒体源没有替换。
新 DLL 的色彩声明变更后，最终安装组件/会议软件的实际解释仍需另行验证。
