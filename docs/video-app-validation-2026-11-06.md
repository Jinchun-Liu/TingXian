# 主窗口视频检查与退出接入（2026-11-06）

当前只提供**显式、无媒体的组件检查**，没有眼部处理、摄像头预览或输出启动命令。
新版 NVIDIA 注册操作已按用户说明暂停；未提交评估申请或创建 Key。后续按用户指令改为
[官方 0.8.7 Ada 独立路线](eye-ar087-acquisition.md)，其下载、许可与本地验收单独记录。
旧数字人继续冻结，基础应用与用户资料未升级、清理或改写。

## 实现与验证决定

`bdd095e` 增加生产检查 worker/Rust 控制器；`cdc0680` 接入主窗、退出握手和打包；
`817a677` 增加真实 Tauri mock IPC；`b47a37ca4688edc67dedf8ac2f519fb928fad7d9` 增加
视频设置、webview 代次校验、已取消派发释放、精确报告上限及测试清单准备。

假设：只有用户点击才会启动固定摘要的检查进程；停止/退出/旧请求不能恢复工作，异常
清理不能冒充正常关闭。决策价值是决定主应用能否采用此控制边界。遇到意外执行、媒体
采集、凭证继承、迟到结果回填或未经确认退出即归档，就停止验收并修复。
结果只影响主窗控制与检查器，不证明 PRD 的摄像头释放时间或眼部功能完成。

worker 只查询系统导出/注册并报告 `runtime: not_pinned`、`media_started: false`。
使用所有权 guard，不调用 session.start、不初始化摄像头/MF、不创建设备、不加载 SDK、
不采音或联网。主应用嵌入构建摘要，运行前锁定文件/目录并核验；可修改的摘要文本不是
信任来源。受限继承 owner/stop/报告句柄、最小环境、隐藏进程、专用 Job 禁止后代进程；
超时只回收自己持有的进程，不按名称终止用户程序。这不是防恶意 EXE 的操作系统沙箱。

主窗 `video_status / video_inspect / video_stop` 检查窗口权限，展示窗全局白名单不变。
检查须带此前读取的 generation，停止先到后旧 IPC 再到也会拒绝。退出先禁止新任务并
等待回收，再进入既有归档；取消退出只恢复准入。文件校验时间不冒称采集释放时间。

“视频”沿用 Mac 0.6.5 控件/颜色；面板挂载只读状态，检查必须点击。停止可越过在途
检查；迟到成功/错误、旧组件回调均隔离。摄像头、预览、视线校正、输出与脸部控件禁用。

## 本机控制器

源码 `cdc0680107fccba7c577f7ae7f84bd75a3d542c1` 的
[原生 CI 42231807163](https://github.com/Jinchun-Liu/TingXian/actions/runs/42231807163) 成功。
artifact 13510116031：1,894,521 bytes；ZIP SHA-256
`c70e7db52f7e83a7a3c566a949b1da1b06972c8d824e2900df3d19e68a05ea0c`。
目标机 22:25 普通用户执行 **12 通过、0 失败、0 跳过**，退出 0，stderr 空，12.62 秒。
测试 EXE SHA-256：`8dd93b604124f74f91a501a1c5bcfede07b2bf3d916809862362653783c7b249`。
涵盖真实生产检查、错误摘要/硬链接、排队停止、退出取消、崩溃、协议错误、超量/卡住
进程、强制回收、无合成秘密继承和禁止后代；不是 Tauri 安装版验证。

22:27 无测试进程残留；原应用 PID 76724、启动时间 2026-11-05 21:11:44、
`D:\Applications\TingXian\meetily.exe` 不变。0.1.2 安装 DLL SHA-256 仍为
`ee541449f2eb41a8bbf58c5f96341a0ead302f611c432c677954855fb840f5d3`。
硬件沿用[同机记录](video-device-validation-2026-11-06.md)，本轮未重测 GPU。
证据在源码外 `artifacts/2026-11-06/video-app-cdc0680/`：完整性清单、stdout/stderr、
`local-result.json`、`local-acceptance.json` 及全部合成夹具。没有创建虚拟设备。

### 最终控制器复核 b47a37c

[原生 CI 42238848921](https://github.com/Jinchun-Liu/TingXian/actions/runs/42238848921) 成功；
artifact 13513675715，1,896,568 bytes，SHA-256
`93286a302136bb0a84971cfe28ae0aea68fb945a0a274b972e5c36bdd3836774`。
23:05 目标机 **12 通过、0 失败、0 跳过**，6.07 秒、退出 0、stderr 空。
测试 EXE SHA-256 `295f96bc38c30044e2f25f6d1b4c4f57adf07e6c3e036057cb6b2b615c7ed99c`。
新增停止后旧 webview 请求拒绝、已取消任务丢弃不误报崩溃的断言通过。
23:06 复核测试进程 0、原 app PID/启动时间及安装 DLL 摘要不变。没有媒体或虚拟设备。
全部夹具和证据在 `artifacts/2026-11-06/video-app-b47a37c/`，包括 `post-check.json`。

## 应用、界面与 CI

- `b47a37c` 本地 **23 组前端脚本、0 失败**；TypeScript、Windows 打包契约及 1569 项
  公开源文件模式扫描通过。新增测试调用真实 React 面板，原生响应采用合成适配器。
- Chrome 实际源码页核对入口、禁用控件、读取失败及详情。2328×1145 无横向溢出。
  浏览器没有 Tauri，其“无法读取视频状态”不是安装版证据；窄窗/缩放/多屏仍待测。
- [42231811069](https://github.com/Jinchun-Liu/TingXian/actions/runs/42231811069) 因增加真实
  IPC 回归而主动取消，终态已确认。没有因轮询失败而重复构建。
- [42232526439](https://github.com/Jinchun-Liu/TingXian/actions/runs/42232526439) 编译成功，
  测试程序在 harness 启动前以 `0xc0000139 / STATUS_ENTRYPOINT_NOT_FOUND` 失败；安装未执行。
  与 [Tauri 上游测试 manifest 说明](https://github.com/tauri-apps/tauri/issues/13419#issuecomment-3398457618)
  一致，须通过导入表/清单及复跑确认。新脚本只给测试副本嵌入 Common Controls v6，
  保存两份摘要、清单、导入表和结果，保留原程序/生产清单与全部 IPC 断言。
- [Windows 构建 42238886149](https://github.com/Jinchun-Liu/TingXian/actions/runs/42238886149)
  在清单处理后成功加载测试程序并执行断言，但主窗 `video_status` 被拒绝，结果为失败，
  安装尚未执行。现有测试沿用 `tauri://localhost`；Windows 的真实本地来源须用
  `http://tauri.localhost`，见 [Tauri 官方 IPC 测试示例](https://docs.rs/tauri/2.11.1/tauri/test/fn.get_ipc_response.html)。
- `3a38411503341eb7eb238865bd462d4661a034c7` 修正视频及原有展示窗测试来源，并收紧
  断言：展示窗必须返回准确权限拒绝，主窗必须返回 off，旧启动必须在代次校验处被拒绝。
  测试清单、生产权限、主窗/展示窗隔离均未削弱。新增展示窗回归与视频回归同次执行。
  [Windows 构建 42242233092](https://github.com/Jinchun-Liu/TingXian/actions/runs/42242233092)
  终态为 success；主窗/展示窗 IPC、CPU 安装包构建、随包检查、篡改拒绝、离线资源及
  CI 安装/卸载通过。仍不代表目标机基础安装版已更新，且该包没有眼部推理功能。

3a38411 完整 CI 的安装包 artifact 13519137033 为 735,039,814 bytes，GitHub 提供的 ZIP
SHA-256 为 `90df6c5f8e42ba8c6da87246b554266345b8a2139501dd4705ef3f3157a6ca1b`；
本轮尚未下载/安装此大包。小型 IPC 证据 artifact 13518472365 已下载，76,709 bytes，
SHA-256 `46b7bab5dd09b385e49621680dac830d82083323e0ce4c1d94fa4c91fb832b20` 与 GitHub
一致，保存于源码外 `artifacts/2026-11-07/app-ci-3a38411/`。新包未覆盖历史发布资产。

第二次失败的证据 artifact 13515740679：76,025 bytes，GitHub SHA-256
`7226f54cf0af886383e0fe7ce9c8876309963e2ba36d530caf362f5f6f433c91`。
本机前两次下载均在 API 请求阶段 EOF，失败文件和 stderr 位于
`artifacts/2026-11-06/video-app-ci-b47a37c/`；尚未把下载失败写成清单内容已复核。

源码 UI 证据在 `artifacts/2026-11-06/video-ui-2245/`。开发服务首次因 Next 遥测配置
重命名 EXDEV 失败；保留日志，仅关闭开发子进程遥测后启动成功，未改系统用户配置。
Git HTTPS 握手失败后，经已有 GitHub 认证/官方 Git Data API 上传，核对 tree/commit
完全相同，按原 SHA 租约非强制快进。artifact 下载的 TLS/EOF 失败也保留，最终摘要
匹配后才执行。安全防护未改，没有上传原始系统跟踪、用户资料或凭证。

23:17 旧仓库仍为 `archive/avatar-frozen-20261106` / `2a80d7b` 且干净，原 app 未重启；
D 盘可用 117,604,433,920 bytes。浏览器检查结束后仅停止经 PID、路径及启动时间核对的
本轮开发服务，截图、日志和全部资料保留。

## 待完成

不重复注册；SDK 工作转入独立旧版路线，本文件不借用其本地结果声明应用集成完成。
继续安装版无声验证；真实摄像头、音画同步、会议接收和 30/60 分钟性能仍需授权及条件。
正常防护下基础 NSIS 的 DLL 写入拒绝仍未关闭；视频 MSI/控制器/CI 不替代其验收。
版本保持实验/测试定位。
