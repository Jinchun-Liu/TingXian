# 可选数字人模块

此目录是独立的 R10/R11 实验组件，尚未接入安装版。导入模块不启动设备、网络服务、LLM 或 TTS。重型运行时和权重放在开发机 D 盘的外部目录，不能进入基础安装包。

`original_voice.py` 把已经重采样的本人原声 PCM16 mono 分成腾讯云 / LiveAvatar LITE 文档规定的请求，并生成结束与打断事件。它不采集、不重采样、不发送、不渲染、不输出音频。`VoicePacket` 的采样位置是输入媒体时间，不能当作“已播放”或“对方已听见”。

`liveportrait_renderer.py` 是已实际出画的独立 CUDA 适配器，接受准备好的正面方形肖像裁切及头姿/眨眼/相对视线参数，返回 512×512 RGB 帧。相对视线控制已用合成照片出画验证，不是用户眼动跟踪或角度校准。身体跟踪、音频口型、摄像头采集和虚拟输出仍为 false；与原声模块尚未连接。它固定校验第三方源码 commit，使用单次 CPU 关键点注册和 GPU 渲染，不静默回落到 CPU 渲染。

`IdleMotion(seed).sample(elapsed_seconds)` 生成无摄像头的合成待机动作：有停留时间的微视线调整、平滑头姿和非固定间隔眨眼。相同 seed/媒体时间不因渲染帧率或跳帧改变；迟到、非有限时间戳拒绝且不推进状态。会话最长 24 小时，重开会话创建新实例。它不是注意力、情绪或同意的测量，也还没有说话/聆听状态融合。`gaze_x/gaze_y` 是 [-1,1] 的相对调整量；不写成眼球角度。所有动作先本地预览，当前自然度仅抽帧检查。

```python
from avatar.original_voice import OriginalVoicePackets

stream = OriginalVoicePackets("tencent", session_id="session-created-by-adapter")
# pcm must be original voice at 16 kHz, PCM16 LE mono, without WAV headers.
packets = stream.push(pcm)
# The future authenticated transport owns scheduling, session isolation and send.
tail = stream.finish()
```

一段声音使用一个实例，结束/打断后禁止继续输入；输入结束后仍允许打断尚在排队的播放。腾讯实例需要会话 ID（由未来的会话适配器取得），LiveAvatar 会话由其 WebSocket 绑定。`push` 每次最多接受 4 秒样本，持续保留不足一片的缓冲；这不是完整音画播放队列。

原声通道直接发送 PCM，不能把 ASR 文本重新 TTS 后冒称原声。原声是否被正确重采样由入口负责。每个发送器须由一个线程/事件循环持有，在发送已生成的包之前检查 session/generation；对端迟到事件也必须隔离。本地接管先停止播放与清缓冲，远端 interrupt 请求仅作为补充。`finish` 是输入结束，不是已经输出确认。

LiveAvatar 默认 1000ms 分片来自上游推荐，不是低延迟目标的通过配置。腾讯实时麦克风为 160ms 分片；离线/TTS 预充策略与真人 1× 输入不能混用。模块目前只实现实时输入的包结构，没有私自调用任何厂商服务。

运行合成回归：

```powershell
python scripts/test-avatar-original-voice.py
python scripts/test-avatar-motion.py
```

协议来源（2026-11-06）：[腾讯音频驱动](https://cloud.tencent.com/document/product/1240/100398)、[LiveAvatar LITE 事件](https://docs.liveavatar.com/docs/lite-mode/events)。市场与选型见 [研究报告](../docs/avatar-research-2026-11-06.md)。

## 本地无声渲染复现

先在 D 盘独立目录取得 `sources.lock.json` 固定版本的官方 LivePortrait 源码。以下命令要求已配置 CUDA 的独立 Python 环境；不要给普通安装用户要求开发工具，也不要修改别的项目环境来凑依赖。本机本轮只读调用已有环境，其版本记录在验收报告；全新环境重建仍待验收。

```powershell
python scripts/avatar-prepare-runtime.py --root D:\TingXian-avatar-runtime --manifest D:\TingXian-avatar-evidence\models.json --with-fixture-generator
python scripts/avatar-generate-fixture.py --model D:\TingXian-avatar-runtime\models\sd-turbo-fixture --output D:\TingXian-avatar-evidence\portrait.png
python scripts/avatar-render-probe.py --source D:\TingXian-third-party\LivePortrait --weights D:\TingXian-avatar-runtime\models\liveportrait --portrait D:\TingXian-avatar-evidence\portrait.png --output D:\TingXian-avatar-evidence\render-01
python scripts/avatar-inspect-video.py --run D:\TingXian-avatar-evidence\render-01
```

在渲染命令增加 `--sequence idle --seed 42` 可生成 10 秒合成待机序列；使用新的输出目录。默认 `controls` 保留原头姿/眨眼测试序列。两种序列的吞吐不能当同素材加速对照。

- 准备脚本只下载公开、固定 revision 的白名单文件，使用 LFS SHA-256 或 Git blob 摘要核验；不安装依赖、不获取凭证。
- 生成合成肖像使用本地 SD-Turbo，推理设置离线，不能把这项工程测试工具描述成已完成个人形象注册。
- 适配器在已裁切方形正面肖像上直接运行上游 landmark 模型，不调用 InsightFace。任意个人照片的检测、自动裁切、质量检查和 UI 仍需实现。
- 输出目录和文件使用新名字；已有结果不覆盖。原始失败记录保留。
- 计时含 GPU 同步和 CPU 取帧，另记录渲染加编码全程吞吐、冷首帧和显存。MP4 标注 25 fps 不等于生成速度达标。脚本不播放视频、不访问相机/麦克风。
- `--cudnn-benchmark` 仅为固定画幅优化对照，未改为默认。早期 23.51/24.61 fps 包含抽帧 PNG 写盘；明确排除该取证开销后的默认配置为 24.79 fps，仍无持续 25 fps 余量。计时修正和原始结果均保留，不宣称实时通过。

模型与代码许可分别见 [第三方说明](THIRD_PARTY.md)。具体结果和限制见 [验收记录](../docs/avatar-validation-2026-11-06.md)。
