# 可选数字人模块

此目录是独立的 R10/R11 实验组件，尚未接入安装版。导入模块不启动设备、网络服务、LLM 或 TTS。重型运行时和权重放在开发机 D 盘的外部目录，不能进入基础安装包。

当前实现仅为 `original_voice.py`：把已经重采样的本人原声 PCM16 mono 分成腾讯云 / LiveAvatar LITE 文档规定的请求，并生成结束与打断事件。它不采集、不重采样、不发送、不渲染、不输出音频。`VoicePacket` 的采样位置是输入媒体时间，不能当作“已播放”或“对方已听见”。

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
```

协议来源（2026-11-06）：[腾讯音频驱动](https://cloud.tencent.com/document/product/1240/100398)、[LiveAvatar LITE 事件](https://docs.liveavatar.com/docs/lite-mode/events)。市场与选型见 [研究报告](../docs/avatar-research-2026-11-06.md)。
