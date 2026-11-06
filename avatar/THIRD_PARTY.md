# 数字人实验的第三方来源

模型核心源码和权重单独下载至仓库外部；本目录只包含下述有明确来源的视线偏移移植，不打包第三方模型或完整运行时。固定版本见 `sources.lock.json`。部署或再分发可选运行时前，需要连同对应许可和通知文件处理，不能将代码许可直接推广到全部依赖。

- **LivePortrait**：官方代码 MIT，Copyright (c) 2024 Kuaishou Visual Generation and Interaction Center。[固定版本 LICENSE](https://github.com/KlingAIResearch/LivePortrait/blob/9b294b3d0536135442ea73cb01e6cb3ca7029dd3/LICENSE)。适配器调用其 wrapper、landmark 和眼部重定向接口，保留原仓库。模型来源为 [KlingTeam/LivePortrait](https://huggingface.co/KlingTeam/LivePortrait/tree/82a4fa6735ca58432b6ce39301b4b9ee066dea47)。本轮白名单不含 InsightFace 的检测权重，也不导入其检测器；任意肖像自动注册不能因此视为已完成。
- **SD-Turbo**：只为工程测试在本地生成完全虚构肖像。[固定模型 LICENSE](https://huggingface.co/stabilityai/sd-turbo/blob/b261bac6fd2cf515557d5d0707481eafa0485ec2/LICENSE.md) 是 Stability AI Community License，不能改标为 MIT/Apache。没有将该模型选为听弦正式形象创建默认模型，也没有打包进入安装器。
- **PyTorch / ONNX Runtime / PyAV / Diffusers 及其依赖**：本轮使用开发机已有安装，未重新分发。基础应用不新增这些依赖；将来独立运行时打包另做完整依赖清单及许可核验。

其他候选（包括带限制条款的 AVTR-1）仍只做来源审阅，未将其实现复制入本模块。

`gaze.py` 的关键点偏移系数移植自固定版本 LivePortrait `src/gradio_pipeline.py` 的 `update_delta_new_eyeball_direction`。保留 Copyright (c) 2024 Kuaishou Visual Generation and Interaction Center，完整上游许可及 InsightFace 说明存于 [LivePortrait-MIT.txt](licenses/LivePortrait-MIT.txt)。听弦新增归一化范围限制、闭眼时衰减及不修改源表情的保护；没有导入上游 Gradio/检测器或启动其网络界面。
