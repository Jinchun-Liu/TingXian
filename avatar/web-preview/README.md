# 听弦本地形象预览

独立、无声的 3D 控制验证页面，支持本地 WAV 推断口型。不会打开摄像头/麦克风、播放音频或调用数字人 API。它没有接入听弦安装版、虚拟摄像头或会议；无声文件预览不等于原声音画同步。

## 启动

需要 Node 22.12+（本机验证 22.14.0）。先将公开 Talking Avatar 源码放到仓库外，例如 D 盘；保留既有 checkout，不回退其他工作。

```powershell
git clone https://github.com/InterviewFlowAI/talking-avatar.git D:\TingXian-third-party\talking-avatar
git -C D:\TingXian-third-party\talking-avatar checkout --detach d3e4dcf6609d733097466eb17b3fa63fc720140a
cd avatar/web-preview
$env:TINGXIAN_AVATAR_SOURCE='D:\TingXian-third-party\talking-avatar'
npm ci --ignore-scripts --no-audit --no-fund --cache D:\TingXian-avatar-runtime\npm-cache
npm run dev
```

打开 `http://127.0.0.1:19270/`。服务只绑定本机地址，启动与构建均检查示例来源提交及 `avatars` / `model` 工作树。初次安装 npm 依赖需要网络；运行时资源均由本机提供，没有 CDN 回退。

## 可验证范围

- 男女 Rocketbox 示例、中文目录下自包含 GLB 导入；文件只在当前页面内存处理，64 MiB 上限，不写用户数据。
- 世界坐标下的左右转头 ±12°、俯仰 ±8°；相对视线、闭眼、三种手动口型、复位、暂停/继续。没有眼动或身体跟踪。
- 控件按模型实际绑定启用。导入模型需提供 `Head`、ARKit `eyeLook*` / `eyeBlink*` 和 Oculus `viseme_*`；本地模型的自动建模、统一比例、任意骨架匹配尚不支持。
- 损坏或外部贴图引用会拒绝，保留当前形象；新选择会隔离旧加载结果。加载 20 秒超时；图形上下文丢失提示刷新。
- 系统减少动态效果时默认暂停；手动调整仍即时出画。使用上游纯 PCM 推断模块，未使用上游播放器或 WebAudio。
- “语音口型”接受 PCM16 单声道 WAV，8–48kHz、最多 60 秒 / 6 MiB；Worker 校验约 1 MB 模型后分析整段。拖动、暂停、继续、清除及结束闭口独立控制，分析可取消。错误文件保留形象。
- 语音仅在页面内存处理；采样位置由无声视觉时钟推进，**不是音频设备时钟**。中文能产生口型序列不代表中文发音准确；当前模型主要用英语训练。

## 无声语音验证

测试同样需要上面的 `TINGXIAN_AVATAR_SOURCE`。回归使用固定上游模型和公开测试夹具，比较其提供的 PyTorch 参考输出；不启动 Python 推理或音频设备。

Windows 可在仓库根目录执行以下脚本，使用已安装的 Huihui / Zira 离线语音生成工程夹具。输出目录必须不存在，避免覆盖证据；生成的是合成测试语音，不是产品替用户说话的入口。

```powershell
.\scripts\avatar-create-speech-fixtures.ps1 -OutputDirectory D:\TingXian-tests\speech-01
cd avatar/web-preview
.\node_modules\.bin\tsx.cmd probe-audio.ts D:\TingXian-tests\speech-01\zh.wav D:\TingXian-tests\zh-report-01.json
```

脚本将 SAPI `AudioOutputStream` 绑定到文件流，不播放结果；此路径见 [Microsoft 文档](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ms723597(v=vs.85))。在预览中导入上述 WAV，检查口型、位置拖动与清除。CLI 报告只包含离线推断计时和口型事件。

模型有 25ms 分析窗、20ms lookahead，事件稳定与重采样还有等待；离线口型平滑读取最多 250ms 范围内的邻近事件并按最终音节时长安排收口。页面已经知道整段未来数据，**不能将分片计算 P95 当成实时或音画延迟**。流式核心单列如下，尚未替换页面的离线模式。

“验证信息”里的十秒结果只计算浏览器动画回调中的 **绘制提交频率/间隔**。手动重绘不计数；暂停、离开前台、换模型会中断，14 秒看门狗兜底。它不含 GPU 完成时间、编码、虚拟设备或远端画面，不能用作 PRD 的 720p / 25fps 会议验收。切换模型清除旧结果。

```powershell
npm test
npm run build
```

构建输出 `dist/` 被 Git 忽略，携带示例资源、口型模型及四份许可。构建不清理旧输出，若需要可分享的归档，应按本次构建 manifest 收集使用中的文件，避免混入旧哈希资源。不要直接对外部署开发服务器。

## 流式原声核心与 1× 无声验证

`OriginalVoiceStream` 接受带 session/generation 和连续采样位置的 PCM16 单声道分片（8–48kHz，每片最多 40ms），只把副本归一化/重采样送入口型推断。输出保留原始 PCM 值、采样率和位置，不使用 ASR、TTS 或声音克隆。调用方提供与输入采样时钟对应的单调 `captureEpochMs`，每次 `pull(now)` 交付最多 20ms 原声及同一媒体位置的口型；该交付不是声卡写入或播放确认。

- 固定目标调度延迟 320ms；环形缓冲最多 400ms，交付超过约定时间 40ms 或数据不足即进入失败，停止交付，不突然补播或伪造静音。已有待交付样本数保留，调用方必须停止输入并提示恢复。
- `VisemeDetector.stableBeforeMs` 标明未来推断不会再回填的事件边界。`stream-mouth.ts` 使用固定起落曲线，借用相邻元音范围 120ms、最大提前起口 95ms，所以最多依赖媒体位置之后 215ms 的已确定事件；当前帧信息未到齐就失败，不窥视文件剩余内容。
- 这是新的流式平滑策略，保留离线策略作对照；缩短借用范围的中文自然度尚未评估。推断、调度、渲染、声卡和远端接收的延迟必须分开记录。
- `finish` 只结束输入，随后仍按时间交付尾部。最终零样本 `end` 标记指定闭口。`cancel` 清空队列，记录 discardedSamples，不能把未播部分计入 deliveredSamples；任何终态都需新 generation 才能重新开始。失败/取消时，未来的设备适配器还须立刻静音和闭口，不能依赖这个纯数据模块停止已排入设备的声音。
- 缓冲有界，旧事件随消费回收。当前单线程 owner 接口，尚未接浏览器 Worker、音频设备、虚拟设备或主应用。

```powershell
.\node_modules\.bin\tsx.cmd probe-stream.ts D:\TingXian-tests\speech-01\zh.wav D:\TingXian-tests\zh-stream-01.json
```

Probe 虽从 WAV 读取夹具，但按墙钟 1× 速度每 20ms 才交给核心；输出只做增量 SHA-256 与时间/口型记录，不播放声音。报告对比输入/交付 PCM 摘要及样本数，保留失败原因和部分交付。`delayP95Ms` 是输入媒体时间到**无声接收器**的交付延迟，不是音画偏移或会议验收；记录包含末尾闭口标记。它未加入 GPU 渲染、ASR 或网络并行负载。

## 来源与限制

`src/vendor/TalkingAvatarRenderer.ts` 基于 [Talking Avatar 固定版本](https://github.com/InterviewFlowAI/talking-avatar/blob/d3e4dcf6609d733097466eb17b3fa63fc720140a/src/renderer.ts)，本地修改增加头部/眼神控制、暂停、来源限制、加载隔离及计时。它是轻量可控形象对照，没有生成用户个人形象。

- Talking Avatar 代码：InterviewFlowAI，MIT；[完整许可](licenses/TalkingAvatar-MIT.txt)。
- `src/vendor/{visemeModel,visemeDetector,resampler,coarticulation}.ts` 为同一固定版本的纯推断与口型映射移植；模型 `model/viseme-model.bin` 由外部 checkout 提供，1,040,276 bytes，SHA-256 `f1cdf2b76b9f2da8730b1d3e47d7e05a2b79c9b92d00dd00c595d64e471f419c`。上游代码与模型均 MIT，权重不进入听弦 Git。
- `visemeDetector.ts` 增加已确定事件边界 getter；`stream-mouth.ts` 沿用同一许可的音素映射，改为有界未来窗口与固定曲线。
- 示例模型：Microsoft Rocketbox，MIT；[完整来源及许可](licenses/Rocketbox-MIT.txt)。由外部固定 checkout 提供，不把 GLB 提交入听弦。
- Three.js 0.186.0：[MIT](licenses/Three-MIT.txt)。包含 GLTFLoader、灯光及环境辅助实现。
- Three.js 内置 meshoptimizer 1.1 解码器：Arseny Kapoulkine，[MIT](licenses/Meshoptimizer-MIT.txt)，[上游许可](https://github.com/zeux/meshoptimizer/blob/v1.1/LICENSE.md)。

实机截图、失败恢复和性能范围见 [验收记录](../../docs/avatar-validation-2026-11-06.md)。个人形象、中文原声口型、同步延迟和虚拟输出分别验收。
