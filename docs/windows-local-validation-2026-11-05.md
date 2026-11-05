# Windows 首轮接手验收 · 2026-11-05

状态：**原包在用户退出 360 后完成普通用户安装，恢复防护主进程后部分无声 GUI 通过；修复版 CI 通过、实机遇到 DLL 拒绝时正确退出 2，但新包完整安装仍阻塞**。使用者明确选择本轮不播放声音、不采集麦克风。测试版定位不变。

## 基线与证据位置

- 私人交接 main：`cd7c3f7`。已完整阅读交接文档与 674 行原始 PRD；公开 PRD 仅替换私人本机链接并增加说明。
- 公开源码：`feat/windows-prd-v2`，接手 HEAD `921654e`，干净 checkout，无本机未提交/未推送历史。当前独立分支 `fix/windows-local-validation-20261105`。
- 安装器源码：`d888e6156201636d1494c4387da295d66095ba1d`。后续 `88729f4` / `921654e` 只修改文档与发布流程，未回退。
- 包：`TingXian-0.7.0-beta.1-Windows-x64-setup.exe`；734767333 bytes；SHA-256 `53a255700ff0bc6c358864a0a8bc39e1c5a8272c470be8eba0713083ac1efbee`。GitHub 元数据及本地完整文件均匹配；Authenticode 状态为未签名，与测试包说明一致。
- [构建运行](https://github.com/Jinchun-Liu/TingXian/actions/runs/41999633117)与[发布核对](https://github.com/Jinchun-Liu/TingXian/actions/runs/42005234413)现场查询均为成功。历史 CI 的 50 项 Windows Rust 核心测试不作为本机重跑结果。
- 原始日志保存在源码仓库外、同级 `artifacts/2026-11-05/`。没有将会话数据库、录音、凭证或桌面原图提交至公开仓库。

## 环境

Windows 11 Pro x64，10.0.26200；Ryzen 7 9700X（8 核/16 线程）；32 GB 物理内存，系统可用总量约 31.1 GiB。RTX 4070 Ti SUPER，驱动 610.88，显存 16376 MiB；另有 AMD 集显和 GameViewer 虚拟显示适配器。未安装或验证 CUDA。

当前一块 G27M2，2560×1440，主屏系统 DPI 96（100%）。未验证其他缩放或第二显示器。采样时可用磁盘约 C: 54.6 GiB、D: 132.4 GiB、E: 90.2 GiB；没有清理任何已有文件腾空间。

枚举到的当前音频端点：G27M2（NVIDIA）、Quantum ES 2 的 Main/Virtual 1 L/R、Virtual 2 L/R、Loopback 1/2 L/R、Inst/Mic/Line 1/2。蓝牙设备的历史枚举项不代表当前已连接。未打开任何音频设备。

执行进程未提升权限。初始定向检查未发现听弦进程、安装登记或兼容应用标识的数据目录；后续发现工具子进程存在 AppData 重定向，因此初始目录枚举只能代表工具所见，不能证明桌面应用目录原先不存在。未终止其他应用或清理其数据。Node 22.14.0 已有；用 `npx pnpm@9.15.9` 安装锁定依赖，保留全局 pnpm 8.15.9。Visual Studio 2022 Community 已存在；本轮未准备 Rust/LLVM/CMake 完整构建环境。

## 试验决策

假设：已发布 CPU 安装包能以普通用户权限完成新装，并在不依赖开发工具的情况下运行内置离线检查。结果决定是否可进入 GUI、音频和真实 AI 阶段。停止条件：安装关键资源缺失、摘要不符、原生自检失败或现有资料受到风险。受影响结论是 Windows 基础安装支持范围，不能以 CI 成功覆盖失败。

默认目录失败后仅追加一次独立中文目录对照，判断是否仅由默认目录造成。两处都缺少关键 DLL 后停止同类静默安装重复，没有忽略错误或复制系统 DLL 来拼补结果。

## 实际结果

| 项目 | 操作与预期 | 实际结果 |
| --- | --- | --- |
| 安装包 | 本地完整大小及 SHA 与交接一致 | 通过 |
| 普通用户默认安装 | NSIS `/S`，默认 `%LOCALAPPDATA%/听弦`，应完整安装 | 失败：进程退出 0，但资源不完整、无完整安装登记；离线自检退出 1，`bundled Python missing` |
| 交互复现 | 查看安装器实际错误 | 明确提示无法写入默认目录 `onnxruntime.dll`；没有选择忽略。自动化无法可靠中止模态后，仅终止了本轮启动且路径/PID 已核对的安装器，保留文件 |
| 独立中文目录 | 非提升权限安装至源码外同级 `installed/` | 同样退出 0，虽生成卸载入口和登记，完整性检查仍失败 |
| 资源核对 | 876 条清单全部匹配，并存在独立 ONNX DLL | 发现 12 个清单内 CRT 文件缺失，另缺 `onnxruntime.dll`；其余清单资源无摘要错误。新增本机诊断一次性列出全部 13 项，退出 1 |
| 部分离线运行 | 原生 `--installation-check`，PATH 移除开发工具 | 文档/Python/FFmpeg 阶段未报错，small 487.01 MB 加载成功；随后 ONNX `LoadLibraryExW failed`，进程 panic 退出 101。整体失败 |
| 可恢复归档/截图 | `--windows-platform-check`，仅合成资料及内存截图 | 退出 0。仅证明本机该自检路径；未验证 GUI 预览、取消、屏幕选择或视觉请求 |
| 前端回归 | `node scripts/test-public-source.cjs` | 本机 21 组，0 失败 |
| 类型检查 | pnpm 9.15.9 `tsc --noEmit --incremental false` | 通过 |
| 包装契约 | `python -X utf8 scripts/test-windows-packaging.py` | 安装配置、摘要篡改及目录越界拒绝通过 |
| 源码公开检查 | `python -X utf8 scripts/check-public-source.py` | 通过；模式扫描不等于完整安全审计 |

缺失清单：`concrt140.dll`、`msvcp140.dll`、`msvcp140_1.dll`、`msvcp140_2.dll`、`msvcp140_atomic_wait.dll`、`msvcp140_codecvt_ids.dll`、`vccorlib140.dll`、`vcomp140.dll`、`vcruntime140.dll`、`vcruntime140_1.dll`、Python 目录中的两个 `vcruntime140` DLL，以及独立 ONNX DLL。

普通空文本及上述 DLL 同名的**非可执行合成写入探针**均能在新的证据子目录创建。因此没有证据声称“磁盘只读”或“所有 DLL 文件名均被禁止”。默认目录在工具进程中出现 Codex LocalCache 重定向，但独立目录也失败，根因仍需定位；不能直接定性为安装包缺文件、杀毒拦截或 MSIX 问题。

## 本轮新增

`scripts/check-windows-local.ps1`：无需系统 Python 的安装后检查；列出全部缺失/修改/越界路径，检查清单外的主程序、卸载器、ONNX、Python 与模型存在性，再运行原生自检。先在当前损坏安装上验证：准确报告 13 项缺失并阻止继续运行自检；没有把退出 0 当作完成。该脚本改善失败诊断，**尚未修复 DLL 写入根因**。

另外用不可执行的合成文件验证：摘要篡改和 `../` 越界清单都被拒绝，未启动任何假程序；已有证据目录拒绝覆盖。续测在完整解包目录验证了脚本成功路径：876 条资源摘要通过，离线检查退出 0；该目录不是安装成功证据。

`docs/windows-read-aloud.md`：来源短测、两分钟复杂长题、本人实话后的追问及 30 题/约 60 分钟顺序，全部合成。尚未朗读或采集。

## 续测：普通桌面启动与解包诊断

假设：对同一发布包做独立完整解包，可区分“包内缺资源”和“安装过程未能写入资源”；普通 Explorer 启动可区分桌面代理重定向与普通应用行为。停止条件为摘要不符、隐式外发、采集启动或真实资料风险。结果影响安装根因定位及首次启动支持边界。

- 用户关闭截图覆盖层后，普通 Explorer 启动的原安装器仍提示无法写入默认目录的 `onnxruntime.dll`。本轮通过模态“中止”正常退出，没有选择“忽略”。默认/独立目录失败均保留；未再反复完整安装。
- 使用官方 7-Zip 将原包完整解压到仓库外 `runtime-diagnostic/`。包内实际包含缺失 DLL；该目录 876 条资源摘要全部匹配，移除开发工具 PATH 后原生 `--installation-check` 退出 0，包括 Python 文档、FFmpeg、small、ONNX/VAD 和合成数据库迁移/恢复。证据：`extracted-validation-2/result.json` 及其 `offline.stderr` / `offline.stdout`。
- 对新合成目录的 DLL 文件名文本写入、实际包内 DLL 的 32/64 位 PowerShell 复制均成功，不能归因为一般目录只读或 DLL 文件名禁写。最小 NSIS 3.08 探针编译成功，但启动返回“拒绝访问”，后续文件已不在原位置；探针未运行，不能提供 NSIS 3.11 安装器的内部错误码。用户回复未看到安全软件提示，事件日志查询未取得相关拦截证据；没有停用防护或改权限，根因仍未确认。
- Codex 子进程启动的解包程序出现首次模型准备 `os error 17`。独立合成探针显示表面 C 盘 AppData 路径实际解析到代理的 D 盘 LocalCache；同目录 `MoveFileExW` 在打开/关闭文件句柄时均返回 17。该探针只创建新的合成文件。经 Explorer 启动后 small 正常就绪，说明此次模型失败与启动上下文有关；没有修改模型复制代码或迁移用户资料。

## 续测：解包程序的无声 GUI

二进制仍是 `d888e615` 的原发布程序。以下结果只适用于完整解包程序、当前单屏 100% 缩放，不代表 NSIS 已安装成功。

| 项目 | 实际结果与边界 |
| --- | --- |
| 正常启动 | Explorer 启动主界面正常；转写授权与 AI 外发均未勾选，开始采集不可用，AI 调用数保持 0 |
| 内置 small | 音频设置显示 `small · 466 MB · 已就绪`、`使用中`；没有下载模型，没有打开音频测试按钮 |
| 设备枚举 | 默认输出、Quantum ES 2 两个输出和 G27M2 可列出；选择 Main/Virtual 1 后显示“设备选择已保存”。仅偏好验证，不等于实际收音 |
| 中文路径 / 五格式 | 新建合成 PDF、DOCX、JSON、Markdown、YAML，扫描 5 项、纳入 5 项、成功 5 项、5 个片段、失败 0，导入耗时 2252 ms；全部关闭片段外发 |
| 结构化预览 | JSON `/value` 显示数字 0，`/optional` 显示 null 空值；JSON/YAML 在未选择字段或整份文件前阻止导入 |
| 本地检索 | `StockMarketAgent 契约测试 null 零 幂等键` 命中五份资料及页/段落/字段出处；状态“仅本地检索 · 未请求模型”，调用数仍 0 |
| 拆分 / 合体 | 创建“听弦 · 对话”展示窗，无输入框；关闭该展示窗后主窗口恢复合体。没有验证 AI 流、滚动跟随、多屏或不同缩放 |
| 重启保留 | 终止空的诊断进程后经 Explorer 重新启动，五份索引、中文根目录、Main/Virtual 1 输出偏好及 small 就绪状态均保留。未测试其他模型切换、正常退出清理或会话快照恢复 |

截图及 UI 文本仅保留在本地：`gui-local-search.png`、`gui-local-search.txt`、`gui-readonly-display.png`、`gui-restart-audio.png` / `.txt`、`gui-restart-library.png` / `.txt`。合成原文位于 `合成资料-无声验收/`，未触碰真实简历/录音。窗口右上角关闭会隐藏到托盘，不能记为程序退出；本轮重启检查在清空已留证的合成检索输入、确认无采集/无对话后，仅终止路径与 PID 已核对的诊断进程，再经 Explorer 启动，不冒充正常退出验收。

源码修正：两个工作区使用后端已有 `os` 字段显示 Windows/macOS/Linux；输入框按 OS 提示 Ctrl/⌘，原有快捷键处理不变。现有输入框回归及 TypeScript 检查通过。当前发布二进制未重建，**修正后的安装版 GUI 尚未验证**。

## 续测：DLL 创建失败的底层返回（20:15—20:36）

用户要求继续定位根因，保持防护开启、暂不采集，后续下载与测试默认位于 D 盘。安装包仍为原 beta.1，摘要未变；当前源码基于 `c043caa`，未回退远端后续提交。

假设与决策：在原 NSIS 3.11 安装器的文件创建返回点记录系统错误，可区分解压损坏、共享冲突和创建拒绝；补充原生调用参数可判定是否为安装器传参错误。每次只使用全新合成目录，首个写入失败即结束本轮拥有的安装器进程，90 秒为上限。仅临时设置该进程的调试断点，未改动安装 EXE、用户资料或防护设置。

| 层级 | 证据 | 结论边界 |
| --- | --- | --- |
| NSIS 文件打开 | `CreateFileW(onnxruntime.dll, GENERIC_WRITE, FILE_SHARE_READ, CREATE_ALWAYS)` 返回无效句柄，`GetLastError = 5` | 拒绝发生于内容写入/解压之前，不是本次被文件占用所报告的共享冲突 32 |
| Windows 原生调用 | `NtCreateFile` 返回 `0xc0000022` (`STATUS_ACCESS_DENIED`) | 原生文件创建被拒绝；仍不足以判定具体驱动或权限规则 |
| 参数对照 | EXE、许可证文本和 DLL 的 DesiredAccess 均 `0x40100080`，属性 `0`，ShareAccess `1`，Disposition `5`，Options `0x60`，ObjectAttributes `0x40`；前两项成功，仅 DLL 失败 | 未发现 DLL 专用访问参数或加密属性异常 |
| 防护信息 | 只读枚举发现多家文件/网络过滤组件；可读的 360 文件日志未匹配到安装器/ONNX，Defender 查询亦无相关事件 | 没有用户可见提示或日志匹配，均不能排除或认定拦截来源；未关闭/排除任何防护 |
| 启动身份 | 工具子进程、Explorer 与安装器的 `GetPackageFullName` 均返回无包身份 15700 | 不能只用父进程名或这一 API 证明已脱离路径重定向；之前实际模型准备的启动差异仍独立保留 |

证据位于仓库外 `artifacts/2026-11-05/trace-installer-open.jsonl`、`trace-installer-native.jsonl`、`trace-installer-parameters.jsonl`，及对应限时诊断脚本。三份失败目录保留。交互安装再次停在同一 DLL 后，通过“中止”进入“安装程序未能完成安装”，未选择忽略。

已从微软下载 Process Monitor 4.12 至 D 盘，Authenticode 为 Valid、签名者 Microsoft Corporation；用户允许管理员诊断授权。后续 Process Monitor 和 WPR 的结果见下节。原始系统日志只保留本地，不上传公开仓库。

另外修正了可独立确认的安装器错误处理：通过 Tauri NSIS hook 设置 `AllowSkipFiles off`，防止静默安装跳过必要文件仍返回成功。CI 增加真实安装器失败回归：在全新合成目录用文件夹占据 `onnxruntime.dll`，断言非零退出且未创建卸载器；然后继续完整安装检查。这是源码修复，等待新构建与实机验证，**不能记为 DLL 拒绝写入已修复**。

补充开发环境：VS 2022 Community 虽存在，本机未发现 C++ `VC/Tools/MSVC` 工具链，不能认为本机已具备构建条件。

## 续测：确认返回拒绝的文件过滤驱动

用户保存的 `../Logfile.PML` 为 1,527,844,408 bytes。读取原始 PML 的进程索引后，只解析本轮安装器 PID 68880 的文件创建事件，避免把其他进程记录上传或误算为目标事件。EXE/许可证创建成功，ONNX DLL 创建为 `0xc0000022`；成功与失败的调用栈均经过 FLTMGR，**这份 PML 调用栈本身不能归责到具体厂商**。有效摘录为 `process-monitor/installer-write-failure-comparison.json`；早先操作码筛选不正确的空摘要不作为证据。

追加一次具有明确决策价值的 WPR 试验：检验文件过滤器的失败回调能否指认拒绝来源。自定义配置只记录进程/模块、文件 I/O 与过滤事件，使用独立会话 `TingXianDll20261105A`，临时文件与 ETL 均位于 D 盘；用户亲自允许管理员记录器。安装器仍为未修改的原发布包、普通用户运行，使用新合成目录；首个失败即停止拥有的安装器，记录器最多 45 秒并自动保存。未卸载驱动、改 ACL、添加信任/排除或关闭任何防护。

| 证据 | 实际值 |
| --- | --- |
| 目标进程与线程 | PID 67384 / TID 62868 |
| 创建失败 | `FileIo/PreOpFailure`，`MajorFunction=0`（创建），`Status=0xc0000022` |
| 返回失败的回调 | `0xfffff802714304a8`，映射到 `360FsFlt.sys + 0x104a8` |
| 模块范围 | 开始与结束的 Image rundown 均为基址 `0xfffff80271420000`、长度 `0xc6000` |
| 驱动身份 | 文件说明“360安全卫士 木马防火墙模块”，版本 `53.0.0.56`，Authenticode Valid；SHA-256 `1c34b37d17745d4829ddd5d3ee7ac011e7ede4cb423f71e6074aef730457076e` |
| 同一进程用户态返回 | 唯一被记录的拒绝写入路径为新目录中的 `onnxruntime.dll`；`NtCreateFile=0xc0000022`、`CreateFileW` 最后错误为 5 |

据此确认：**本次普通用户安装的 DLL 创建由 360FsFlt 的前置回调返回拒绝**。这不是按驱动安装清单推测。尚未取得产品规则名称；也不能仅据此把所有历史缺失文件逐个归因，或断言命中了哪条恶意行为/误报规则。

原始完整 ETL 为 `process-monitor/minifilter-capture-1/installer-minifilter.etl`（113,306,599 bytes）。派生 XML 转换因体积过大主动停止，仍保留原 ETL；提取的 6,421,028 个完整事件已包含目标失败和模块开始/结束记录。`target-filter-events.json` 是局部派生证据，不能用于证明未发生其他事件。文件对象地址会复用，归因采用进程/线程、唯一拒绝事件与同次原生调用对照，未把相同地址的较早目录打开误当作 DLL 打开。

用户随后主动报告已关闭 360；代理没有执行关闭操作。现场未见 360 主进程，但 `360FsFlt`、`360Box64`、`360AntiSteal` 仍报告 Running，因此不把“退出界面”当作驱动卸载。在全新 `D:/Applications/TingXian` 完成一次条件变化后的普通用户安装对照，原失败目录和应用资料保留；这不能替代防护开启时的安装验收。

## 续测：用户退出 360 后的安装版

假设：在用户主动改变防护产品状态后，原包可正常落盘；这一对照能区分当前产品拦截条件与不可变的安装包缺陷。仅运行一次、全新目录、120 秒上限；资源缺失/摘要不符即失败。安装进程明确断言非提升权限，TEMP/TMP 和证据目录均在 D 盘，未手工补 DLL。

- 原包 SHA-256 不变、源码仍为 `d888e615`。20:52:34 开始，20:53:12 退出 0，实际安装位置 `D:/Applications/TingXian`；HKCU 卸载登记指向新位置。原 `installed/meetily.exe` 仍存在，之前的登记已单独导出保存，未卸载/清空旧失败目录。
- 安装后 876 项清单摘要全部一致，缺失/修改/越界均为 0；主程序、卸载器与清单外 ONNX 也存在。原生离线自检退出 0，涵盖包内文档依赖、FFmpeg、small、ONNX/VAD、合成数据库检查；子进程 PATH 不含开发工具。
- 这是**真实安装后的有条件通过**，不再只是完整解包诊断。但安装时 360 产品状态被用户改变，不能写成“保持原防护设置安装通过”。20:56 再次枚举到 360Safe / 360tray 运行后，安装版 876 项摘要和离线自检再次通过（`installed-restored-360-validation/result.json`）。进程运行不等于完整核验所有防护开关；防护开启时重新安装仍未验收。
- 用户重新授权并再次调用电脑插件后，工具仍返回物理 Esc 停止状态，没有恢复桌面连接；未再进行界面输入。安装版 GUI 启动、small 设置页面、资料恢复和正常保存退出仍待验证，不能引用之前解包程序的截图来填补。本轮仍未采集音频或调用 AI。

证据：`artifacts/2026-11-05/install-after-user-exit-360/installation.json`、`validation/result.json`、`validation/offline.stdout` / `.stderr`。源码 `fb5b8f7` 的新包仍单独构建，不能把原包本次结果记到新包上。

## 续测：安装版 GUI 与新包 CI（21:00 后）

用户在新一轮重新调用电脑插件后，桌面连接恢复。试验假设为同一原发布二进制的安装版能够复用既有资料/偏好并正常显示 small；结果决定能否继续安装版交互验收。遇到采集启动、隐式 AI 外发或未保存真实输入则停止。本轮只读取设置及合成资料，未点击音频测试或授予采集/外发。

- 通过 Explorer 启动 `D:/Applications/TingXian/meetily.exe`，进程路径现场核对一致，主界面正常，状态就绪、AI 调用 0，转写授权/AI 外发未勾选，开始聆听禁用。
- 安装版音频设置显示 `small · 466 MB · 已就绪`、`使用中`；Main/Virtual 1 输出选择保留，两个音频测试状态均为“未测试”。
- 安装版资料库恢复此前 5 份合成文件和 5 个片段，中文资料目录一致；会议记录显示无已保存会话、无旧会议。
- 原包仍显示 macOS / ⌘ 的已知提示错误，未把修正源码误认为已安装。当前 GUI 证据仅针对 `d888e615`。
- 插件未返回可操作的托盘窗口；用户按提示从托盘选择“退出听弦”，随后进程枚举确认退出。只验证了空会话的正常退出，不能扩展为含对话/草稿的保存退出。
- 退出后命令行备份至仓库外 `before-preview-install/tauri-roaming`，8 个文件共 487,984,792 bytes，逐文件摘要与该进程所见源目录一致。后续文件句柄核对确认其实际来自 Codex 的 LocalCache/Roaming，**不是桌面应用的完整资料备份**；更正见下节。备份及数据库只保留本机。

GUI 证据位于 `gui-installed-original/01-home.png` / `.txt`、`02-audio-settings.txt`、`03-restored-library.txt`。

源码 `fb5b8f7c73f4839463787ab62b7bd05e428cf18a` 的 [CI 42027994092](https://github.com/Jinchun-Liu/TingXian/actions/runs/42027994092) 已成功完成：前端/公开源码/包装检查、50 项 Windows Rust 核心测试、CPU 打包、真实安装器失败回归（必要 DLL 路径被合成目录占用时退出 2）、876 项安装摘要、离线运行时、合成平台检查和卸载保留资料均通过。新包为独立分支的 CI artifact，未覆盖 beta.1 发布资产，未提升为稳定版。实机新包验收另记。

## 续测：修复版实机失败处理

下载与临时文件均在 D 盘。首次 `gh run download` 无数据进展，停止该下载后使用既有 GitHub 登录重取，未输出或保存凭证/签名 URL。CI artifact ZIP 734,994,981 bytes，SHA-256 `bbaa39fd2ebde9707cba3ad2b3073b135653bbcdb5ca230b03bd33615e5e0dc2`，与 GitHub artifact 元数据一致。

新安装器仍使用 beta.1 文件名，但仅位于独立 `ci-fb5b8f7/` 证据目录：734,786,779 bytes，SHA-256 `29e984d15ce2e24b880937522384e3b75b01f3b3a3f8bbccd8546104ee9778d4`，与该 artifact 内 SHA256SUMS 一致；源码为 `fb5b8f7`，未签名。不得与 734,767,333 bytes 的原发布包混用。

假设：新包遇到本机 DLL 拒绝时应明确失败并保留原完整安装；若完成则必须通过资源/离线检查。决策为验证失败处理修复的实机行为；仅一次全新目录，180 秒上限，不改防护、不手工补文件。

- 21:09:04—21:09:22，普通用户安装至 `D:/Applications/TingXian-preview-fb5b8f7`；开始前 360Safe / 360tray 均在运行，未改其设置。
- 实际退出 **2**，`onnxruntime.dll` 与 `uninstall.exe` 均未落盘。失败目录保留，不启动该部分安装。
- 同一命令行上下文读取的 HKCU 卸载登记仍指向完整的 `D:/Applications/TingXian`，显示版本/卸载命令/路径均未改变；该上下文所见 8 个资料文件摘要与安装前副本逐项一致。它们后来被确认属于 Codex 缓存副本，不能据此推断桌面资料逐文件未变；实际桌面应用重启后仍恢复五份合成资料并可检索。
- **安装失败返回正确错误码的修复通过实机验证；新包完整安装未通过。** 这次没有重复 WPR，不能把前一包的内核事件写成新包重新采样结果；新包失败形态与此前一致，原防护状态下的新装仍是待处理的兼容性问题。

证据：`ci-fb5b8f7/verified.json`、`workflow.log`、`preview-fb5b8f7-install/installation.json`。已在本地准备 `360-compatibility-report-draft.md`，尚未提交或上传；原始 PML/ETL 不进入公开日志或厂商反馈草稿。[360 官方反馈页](https://open.soft.360.cn/report.php)现场显示文件上限 512 MB，并要求短信验证码和邮箱。完整安装器超过此表单限额；后续找到官方大文件链接受理说明，见下一节，不再把表单限额当作唯一渠道。

## 续测：桌面资料路径与合成回收核对

原完整安装版重启后，本地检索 `StockMarketAgent 契约测试 null 零 幂等键` 返回五份资料及页/段落/字段来源，状态“仅本地检索 · 未请求模型”、调用数 0。独立展示窗只有对话展示区，无输入控件；关闭后恢复合体，仅剩主窗口。证据为 `gui-installed-original/04-restart-local-search.png` / `.txt`、`05-readonly-display.jpg` / `.txt`。这些结果属于原发布源码 `d888e615`，不属于未完成安装的新包。

合成索引清理前的只读核对发现：界面为 5 份资料，而命令行读取的库为 0。文件句柄最终路径显示，C/D 盘用户 Roaming 路径均被重定向至 Codex 包的 `LocalCache/Roaming`。因此此前 8 文件备份及摘要比对只保护了工具上下文的缓存副本，不是桌面应用资料的完整备份；不能以此证明桌面数据在升级前后逐字节未变。发现差异后未执行清理，先纠正备份。

通过普通 Explorer 从桌面应用 Roaming 目录复制 `interview` 至 D 盘 `desktop-data-backup/`。应用当时无采集/导入/会话写入，副本只包含 `profile.sqlite`，SQLite 完整性检查为 `ok`。核对其根目录及五个相对文件名与本轮合成原文完全一致，documents/chunks/search 均为 5，document_metadata/background_library 均为 0。该副本是索引备份，不宣称是完整会话/凭证备份。

试验假设为安装版清理索引会先生成可读的回收归档并保留原文；结果决定清理功能是否具备实机证据。仅对核对过的五份合成资料进行一次清理；备份、原文或恢复异常立即停止。

- GUI 清理后显示 0 份资料、0 个片段，回收归档存在且可解析，类型 `meetily-profile-index-backup`、格式版本 2。
- 归档的五张索引表逐行与实际桌面库副本一致；五个合成原文件 SHA-256 与清理前一致。未清空回收站、未清理会话历史或真实资料。
- 归档摘要 `cb2330bcb6827ef8cc9deb90c62d57a8b2560c1aeeaba3aa6521f195035d786b`。证据为 `desktop-data-backup/verified.json`、`recycle-verified.json`、`gui-installed-original/06-index-cleared.jpg` / `.txt`；数据库和包含本机标识的截图仅本地保存。
- 安装版重新扫描五格式原文，JSON 预览分别显示数字 0 与 null 空值，JSON/YAML 未选字段时导入按钮禁用。嵌套滚动中的自动化定位偏差曾先导入四份合成文件；记录该中间状态后补齐 YAML，并关闭五份文件的片段外发。最终报告为 discovered/included/success 均 5、unchanged 4、excluded 0、chunks 5、failures 空、耗时 605 ms、索引版本 4（`07-reimport-result.txt`）。这是从保留原文重新导入，不是回收站归档回导或会话恢复验收。
- 恢复后再次执行同一合成本地查询，五种格式及页/段落/字段来源全部返回，AI 调用仍为 0；证据 `08-recovered-local-search.jpg` / `.txt`。应用保持运行且无采集，未退出或丢弃输入；输入框只保留本轮已留证的合成检索词。

## 续测：无可见拦截记录与 ONNX 来源复核

用户已检查 21:09 左右的产品防护记录，回复“未找到相关条目”。记录为产品界面未提供规则信息，不能据此排除同日原包的 WPR 归因。尚未取得规则名或厂商处置结果；没有再次运行同条件安装、关闭防护或添加信任。

假设：新安装包的 ONNX 组件与项目锁定的上游文件一致。该核对决定是否需要修复构建取包/打包链；三个已声明组件任一不符即停止并定位。全部一致则停止重复重打包试探，转向产品规则核查。每个组件最多 90 秒，只在内存读取 NSIS 内容，不把 DLL 写入失败安装目录、不运行新程序。

| 核对 | 实际结果 |
| --- | --- |
| 新包 `onnxruntime.dll` | 12,418,080 bytes，SHA-256 `579b636403983254346a5c1d80bd28f1519cd1e284cd204f8d4ff41f8d711559`；与 `build/onnxruntime.rs` 的上游锁定值、原完整安装中的文件逐字节一致 |
| 共享 DLL / 许可证 | `onnxruntime_providers_shared.dll` 与 `onnxruntime-LICENSE.txt` 的大小及 SHA-256 也全部匹配源码锁定值和原完整安装 |
| 原完整安装中首个 ONNX DLL | Authenticode `Valid`，发布者 Microsoft Corporation，文件版本 `1.22.20250508.2.f217402` |
| 应用 / 新安装器 | `NotSigned`，与测试版配置一致；没有证据证明“未签名”就是本次 360 规则条件，不能以购买或自签证书替代归因 |
| 原公开发布资产 | GitHub 元数据再次核对大小 734,767,333 bytes 及原 SHA-256 一致，取得不需要登录的公开下载地址 |

以上证据支持“组件未在此打包链中被改写”，不等于安全软件已审核、没有任何风险或安装已修复。上游来源为 [Microsoft ONNX Runtime v1.22.0](https://github.com/microsoft/onnxruntime/releases/tag/v1.22.0)；项目锁定压缩包及逐文件摘要，未临时换版本。

[360 官方上传须知](https://open.soft.360.cn/guide/guide_2.php?i=3)允许较大软件提供最终文件下载链接，并要求避免需注册登录的中转地址。已据此准备原发布资产的直接链接及脱敏邮件正文，请厂商确认大文件受理渠道、拒绝规则与保持防护开启的处置办法。原始 PML/ETL、个人资料、会话数据库和凭证均不包含在待审阅材料中；未经用户确认不发送邮件。反馈准备不等于已受理或已解除拦截。

本地证据：`provenance-followup/verify-payload.py`、`payload-verification.json`、`signatures.json`、`public-release-asset.json`。可审阅正文为 `360-feedback-email.txt`；材料 ZIP `360-review-materials-20261105.zip` 含正文与两份摘要 JSON，共 4,191 bytes，SHA-256 `649ae20bc1dccb9dbee9baf21eaa78fc4b8562a3fb870b8d5c046e13960ca39a`，逐项检查及 ZIP 完整性核对通过。该 ZIP 是诊断材料，不是替代安装包。听弦仍在原完整安装目录运行，本轮未操作 GUI、音频或 AI。

## 待继续

1. 已确认 360FsFlt 回调返回拒绝；用户未找到产品界面条目，组件来源核对通过。取得用户外部联系授权后向官方核查规则与处置方法，再验证普通用户安装。保持原失败目录和日志，不能把关闭防护后的对照算成防护开启时通过。
2. 新包已构建并校验；解除产品拦截原因后验证完整普通安装、修正后的 GUI 系统提示、音频模型和保存恢复。当前原安装版的通过项不能代替新包验收。
3. 补齐合成资料卸载保留、重装及旧格式兼容。原安装版索引回收归档核对已通过，含会话的保存退出仍未验证；任何后续升级前先核对备份的实际路径和内容。
4. 使用者允许后开展本地系统声/麦克风分流、独立开关、三次停止再启与清空后再启，保持 AI 调用数 0。
5. 使用者在应用中配置自己的接口并允许外发后，完成三路不同问题、排队、取消、故障与迟到隔离、真实上下文和回答质量；再推进截图视觉、多屏、五种模式、背景整理/Skill 回导、256K 边界及双设备长会议。

未做：真实 Windows 音频、GUI 全流程及会话保存退出、卸载重装、AI/视觉接口、第二显示器、USB/蓝牙拔插/采样率/权限拒绝、30 题/60 分钟、无高端独显机器、CUDA、云 ASR、数字人和代理。
