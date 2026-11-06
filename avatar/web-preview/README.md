# 听弦本地形象预览

独立、无声的 3D 控制验证页面。不会打开摄像头/麦克风，也不调用数字人 API。它没有接入听弦安装版、虚拟摄像头或会议；手动选择口型不等于原声音画同步。

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

打开 `http://127.0.0.1:19270/`。服务只绑定本机地址，启动与构建均检查示例来源提交及 `avatars` 工作树。初次安装 npm 依赖需要网络；运行时资源均由本机提供，没有 CDN 回退。

## 可验证范围

- 男女 Rocketbox 示例、中文目录下自包含 GLB 导入；文件只在当前页面内存处理，64 MiB 上限，不写用户数据。
- 世界坐标下的左右转头 ±12°、俯仰 ±8°；相对视线、闭眼、三种手动口型、复位、暂停/继续。没有眼动或身体跟踪。
- 控件按模型实际绑定启用。导入模型需提供 `Head`、ARKit `eyeLook*` / `eyeBlink*` 和 Oculus `viseme_*`；本地模型的自动建模、统一比例、任意骨架匹配尚不支持。
- 损坏或外部贴图引用会拒绝，保留当前形象；新选择会隔离旧加载结果。加载 20 秒超时；图形上下文丢失提示刷新。
- 系统减少动态效果时默认暂停；手动调整仍即时出画。没有加载音频推断模型，未使用上游音频播放器。

“验证信息”里的十秒结果只计算浏览器动画回调中的 **绘制提交频率/间隔**。手动重绘不计数；暂停、离开前台、换模型会中断，14 秒看门狗兜底。它不含 GPU 完成时间、编码、虚拟设备或远端画面，不能用作 PRD 的 720p / 25fps 会议验收。切换模型清除旧结果。

```powershell
npm test
npm run build
```

构建输出 `dist/` 被 Git 忽略，携带示例资源及四份许可。构建不清理旧输出，若需要可分享的归档，应按本次构建 manifest 收集使用中的文件，避免混入旧哈希资源。不要直接对外部署开发服务器。

## 来源与限制

`src/vendor/TalkingAvatarRenderer.ts` 基于 [Talking Avatar 固定版本](https://github.com/InterviewFlowAI/talking-avatar/blob/d3e4dcf6609d733097466eb17b3fa63fc720140a/src/renderer.ts)，本地修改增加头部/眼神控制、暂停、来源限制、加载隔离及计时。它是轻量可控形象对照，没有生成用户个人形象。

- Talking Avatar 代码：InterviewFlowAI，MIT；[完整许可](licenses/TalkingAvatar-MIT.txt)。
- 示例模型：Microsoft Rocketbox，MIT；[完整来源及许可](licenses/Rocketbox-MIT.txt)。由外部固定 checkout 提供，不把 GLB 提交入听弦。
- Three.js 0.186.0：[MIT](licenses/Three-MIT.txt)。包含 GLTFLoader、灯光及环境辅助实现。
- Three.js 内置 meshoptimizer 1.1 解码器：Arseny Kapoulkine，[MIT](licenses/Meshoptimizer-MIT.txt)，[上游许可](https://github.com/zeux/meshoptimizer/blob/v1.1/LICENSE.md)。

实机截图、失败恢复和性能范围见 [验收记录](../../docs/avatar-validation-2026-11-06.md)。个人形象、中文原声口型、同步延迟和虚拟输出分别验收。
