# 发布产物静态验收与环境边界

日期：2026-10-31，更新于 2026-11-01（Asia/Shanghai）。本记录只检查本地构建和产物，不发布、不执行安装器、不修改本机注册表或真实客户端数据。

**当前状态：包含透明进度悬浮窗数字、颜色与任务行显示微调的 release round7 已实际构建 exit 0，并在全新 TEMP 目录通过 13/13 静态验收。round6/5/4/3/2 均为历史阶段候选；round6 不包含本次前端微调。本报告没有执行客户端或安装器，没有发布资产；原生启动、固定端口健康接口、悬浮窗原生行为及安装生命周期仍未验证。**

## Round7 实际候选（当前）

本轮 `pnpm tauri build` 的 UTC 起点为 **`2026-10-31T17:16:43.7650822Z`**，结束 `2026-10-31T17:20:44.6162905Z`。根任务实际读取 session 97248 的 **exit 0**，并核对落盘 [release-round7-result.json](C:/Users/20269/AppData/Local/Temp/maobu-visual-polish-0644ba78dcc64d63bf7451fc4e221f42/release-round7-result.json)；日志为 `%TEMP%/maobu-final-release-round7.log`。

确认构建成功后，`2026-10-31T17:21:22.5947159Z` 开始全新目录验收：**13/13 通过，exit 0，STATIC_CANDIDATE_CHECKS_PASSED**。机器可读证据为 [round7 release-inspection.json](C:/Users/20269/AppData/Local/Temp/maobu-release-round7-fcbb0e37a6494db986ea5a528fa19873/release-inspection.json)，同目录保留五份实际产物、提取内容、7-Zip 列表、只读 MSI 表及 Get-FileHash 校验表；指针为 `%TEMP%/maobu-release-round7-output.txt`。

本轮仅修改两份前端源：进度页局部样式与任务行显示，包括数字衬底、主题轨道颜色、文件名字重、整数百分比、速度对齐及校验等阶段的状态表达；窗口尺寸、原生窗口和下载逻辑未改。293 份 Rust 源 SHA 与 full15 一致，根任务已核查 Changed=0，**本轮没有重跑 cargo**；沿用此前 full15 的 1497 通过/0 失败/16 忽略及 Range 9 通过，不能将其写成 round7 新执行的测试。前端 check/build/extension:build 已通过；核心 UI **22 组/18 图**、宿主背景 **6 组/12 图**为实际 React 的隔离浏览器/API 与原生尺寸延迟替身，背景为测试页面内容，不代表原生 Windows 宿主。详情及证据见[视觉细调专项](2026-11-01-progress-window-polish.md)。

App/NSIS/MSI 的 UTC 修改时间分别为 `17:20:44.5893318Z`、`17:20:40.9854766Z`、`17:20:42.9850000Z`，均晚于本轮起点。以下字节数及 SHA 由新候选实测；即使部分字节数与 round6 相同，也未沿用其哈希。

| Round7 当前产物名称 | 实际字节 | Get-FileHash SHA-256 |
| --- | ---: | --- |
| maobu-fetch-v0.10.11-portable.exe | 14182912 | 4febfd4928a3e59bba4fc0c7f1867a0ca618c9e99e25f7c6d8487bcb31986b75 |
| Maobu Fetch_0.10.11_x64_zh-CN.msi | 7090176 | af1475d52a652f767309a0b2f218f51e988dd16cb7bcc398acc437774b22dc18 |
| Maobu Fetch_0.10.11_x64-setup.exe | 5318249 | a1bedd4e8849e1f031d4af4b96fbb809088f05514515f13c3d2b1dfc63d00d0e |
| maobu-fetch-extension-v0.10.11.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |
| extension.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |

五处版本仍为 `0.10.11`，旧 identifier=`app.lumaget.desktop`。实际 PE ProductName=`Maobu Fetch`、ProductVersion/FileVersion=`0.10.11`、OriginalFilename=`maobu-fetch.exe`；MSI ProductVersion=`0.10.11`，File 表只含目标主程序，cabinet ID `Path` 正确映射。NSIS/MSI 均严格低于 30,000,000 字节，实际清单不含三个媒体工具可执行文件。

7-Zip `26.02` 实际提取 NSIS **8 文件**（主程序、卸载程序、两张位图、四个插件 DLL），ParserWarnings 为空；MSI 提取 1 个主程序。双扩展包各 **39 文件**，当前非测试源/manifest 与 dist 逐文件 SHA 对齐，无测试脚本，两个别名 ZIP 完全同字节。卸载程序只提取、未执行。

三份主程序均为 14,182,912 字节。完整唯一 Tauri 标记验证后，严格仅归一十进制偏移 **`12890992`** 的 3 字节 bundle 类型，两份归一后的整文件 SHA 均与便携 EXE 一致。原始未修改的 SHA 单独保留：

| Round7 主程序来源 | 未修改的实际 SHA-256 |
| --- | --- |
| 便携 EXE（UNK） | 4febfd4928a3e59bba4fc0c7f1867a0ca618c9e99e25f7c6d8487bcb31986b75 |
| MSI 提取 EXE（MSI） | 850b88b17569fe787cb2b954d4a59d360080201275396590353770f94e5a73c7 |
| NSIS 提取 EXE（NSS） | f4d01ab0b9410b7cc9bc855af4ded7215e03b96ff3f0e6b32d2c62beffddfe3e |

程序与全部安装包/扩展提取文件的本机 workspace/profile 路径扫描命中 **0**，覆盖 UTF-8、UTF-16LE 偶/奇对齐及两种斜杠。该静态验收不证明 release EXE `/v1/health`、NativeSink 提示显示/关闭、Windows 透明合成/置顶/焦点/多屏 DPI/点击穿透、安装/升级/回滚/卸载或 Windows 10 真机兼容。本轮未恢复桌面控制，未操作旧客户端、17433、个人数据库、设置、Cookie 或宿主机注册表。

## Round6 实际候选（阶段历史）

本轮 `pnpm tauri build` 的 UTC 起点为 **`2026-10-31T16:21:14.7586040Z`**，结束 `2026-10-31T16:28:09.1354002Z`。根任务实际读取 session 27359 的 **exit 0**，并核对落盘 [release-round6-result.json](C:/Users/20269/AppData/Local/Temp/maobu-final-round15-b5773ea817bc4c7baf806221a48489e8/release-round6-result.json) 的 ExitCode=0；日志为 `%TEMP%/maobu-final-release-round6.log`。未从日志完成字样或失效句柄推定退出码。

构建明确成功之后，`2026-10-31T16:54:16.5142627Z` 开始全新目录验收：**13/13 通过，exit 0，STATIC_CANDIDATE_CHECKS_PASSED**。机器可读证据为 [round6 release-inspection.json](C:/Users/20269/AppData/Local/Temp/maobu-release-round6-27a39dd6686046d3990adef91004e332/release-inspection.json)，同目录保留五份实际产物、提取内容、7-Zip 列表、只读 MSI 表及 Get-FileHash 校验表；指针为 `%TEMP%/maobu-release-round6-output.txt`。没有复用旧轮验收目录或旧产物值。

本轮构建源包含常态 **224×28** 细进度条、悬停详情 **224×190 / 224×298**、透明窗口/WebView 背景、底部锚点及真实字节加权汇总。相应源码回归为完整 round15 **1497 passed / 0 failed / 16 ignored**（114.58 秒），另 Range **9 passed / 0 failed**（5.06 秒）；日志实际统计已复核。根任务核查 293 份 Rust 与 115 份前端源快照 Changed=0。界面专项的 21 组/18 图和透明 PNG 证据见[极简透明浮窗专项](2026-10-31-compact-progress-window.md)；这些浏览器/静态证据不代替原生 Windows 窗口的透明合成、显示、置顶与焦点验收。

App/NSIS/MSI 的 UTC 修改时间分别为 `16:28:09.1016812Z`、`16:28:05.1314159Z`、`16:28:07.3860000Z`，均晚于本轮起点。下表由新文件实测，未沿用 round5 的大小或 SHA。

| Round6 阶段产物名称 | 实际字节 | Get-FileHash SHA-256 |
| --- | ---: | --- |
| maobu-fetch-v0.10.11-portable.exe | 14182912 | e930a73e7979edef715bb9b26f293492ad3437c77f87005166485d6cb977dc81 |
| Maobu Fetch_0.10.11_x64_zh-CN.msi | 7090176 | ac0e0a23c19196b1b0bc76a4c38a71891f1917098e0e1c22cc4caebe78808620 |
| Maobu Fetch_0.10.11_x64-setup.exe | 5311319 | 6815e97914ddb4450e1f130d772bd496ab3669629834b54b4654f57ff30b0274 |
| maobu-fetch-extension-v0.10.11.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |
| extension.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |

五处版本仍为 `0.10.11`，旧 identifier=`app.lumaget.desktop`。真实 PE ProductName=`Maobu Fetch`、ProductVersion/FileVersion=`0.10.11`、OriginalFilename=`maobu-fetch.exe`；MSI ProductVersion=`0.10.11`，File 表中只有目标主程序，cabinet ID `Path` 正确映射。NSIS/MSI 均严格低于 30,000,000 字节，实际清单不存在三个媒体工具可执行文件。

7-Zip `26.02` 提取 NSIS **8 文件**（主程序、卸载程序、两张位图、四个插件 DLL），ParserWarnings 为空；MSI 提取 1 个主程序。两个扩展包各 **39 文件**，当前非测试源/manifest 与 dist 逐个 SHA 对齐，无测试脚本，双别名 ZIP 完全同字节。未执行卸载程序。

三份主程序均为 14,182,912 字节。完整唯一 Tauri 标记验证通过后，仅归一十进制偏移 **`12890848`** 的 3 字节 bundle 类型；两份归一后的整文件 SHA 均与便携 EXE 相同。未修改的原始 SHA 如下：

| Round6 主程序来源 | 未修改的实际 SHA-256 |
| --- | --- |
| 便携 EXE（UNK） | e930a73e7979edef715bb9b26f293492ad3437c77f87005166485d6cb977dc81 |
| MSI 提取 EXE（MSI） | 6c51a409ab9e5dc9f3930da841a6a2d97a211562afddc5f7a1e1071e7fc5366c |
| NSIS 提取 EXE（NSS） | 854bb80918c2b4442387627f7866f70df69bced8db2c1ce4af21e5424d3359d3 |

实际程序及全部安装包/扩展提取文件的 workspace/profile 路径扫描命中 **0**，覆盖 UTF-8、UTF-16LE 偶/奇对齐及两种斜杠。上述 13 项证明本轮候选的静态包结构、元数据、大小和内容一致性；不证明真实 release EXE `/v1/health`、NativeSink 提示显示/关闭、透明悬浮窗原生行为、安装/升级/回滚/卸载或 Windows 10 真机兼容。本轮未操作旧客户端、17433、个人数据库、设置、Cookie 或宿主机注册表。

## Round5 实际候选（阶段历史）

早先 round5 构建日志已完整输出 `Finished release`（6 分 18 秒）与两份 bundle，但旧 exec handle 后续已失效，且未单独保存退出码；不能把该历史记录写成已恢复的 exit 0。本次在未改源的基础上重新执行核定构建：UTC 起点 **`2026-10-31T15:13:30.9942952Z`**，结束 `2026-10-31T15:17:34.7376647Z`，真实 **ExitCode=0** 已写入 [release-round5-confirm-result.json](C:/Users/20269/AppData/Local/Temp/maobu-final-round14-a8ab0dc3fb3943d6b6a3d90268c1b608/release-round5-confirm-result.json)。日志为 `%TEMP%/maobu-final-release-round5-confirm.log`。本轮验收仅使用核定构建的新起点，不复用先前起点或先前安装包大小。

确认构建成功之后，`2026-10-31T15:25:31.3661997Z` 开始 fresh 验收：**13/13 通过，exit 0，STATIC_CANDIDATE_CHECKS_PASSED**。机器可读证据为 [round5 release-inspection.json](C:/Users/20269/AppData/Local/Temp/maobu-release-round5-bb4c91439d344c1d8f044df08133b8c1/release-inspection.json)，同目录保留五份实际产物、提取文件、7-Zip 列表、只读 MSI 表与 Get-FileHash 校验表。目录由验收脚本首次创建，指针为 `%TEMP%/maobu-release-round5-output.txt`。

本轮包含下载进度悬浮窗、置顶设置持久化、后台创建 Windows WebView、蓝奏明确二进制文件的 HTML 响应拒绝，以及此前全部修复。根任务完整 round14 日志实际为 **1494 passed / 0 failed / 16 ignored**（113.34 秒），另 Range **9 passed / 0 failed**（5.06 秒）；源码快照为 292 份 Rust 文件，根任务核查 Changed=0。保留此前 full13 栈溢出中止的历史和根因尚未唯一确定的边界；后续完整测试通过不覆盖该事实。单元/聚合和静态检查不代替悬浮窗实际显示、置顶与焦点等原生行为验收。

App/NSIS/MSI 的 UTC 修改时间分别为 `15:17:34.7132068Z`、`15:17:30.8984260Z`、`15:17:33.0990000Z`，均晚于核定起点。以下值来自最新实际文件，不使用早先 NSIS 的 5,310,870 字节记录。

| Round5 阶段产物名称 | 实际字节 | Get-FileHash SHA-256 |
| --- | ---: | --- |
| maobu-fetch-v0.10.11-portable.exe | 14176768 | ace153d4075ae62838c48525f331d8fcc9af987469a17f3abde6b50c65b1851d |
| Maobu Fetch_0.10.11_x64_zh-CN.msi | 7086080 | 06f700817765e86632cfcb5d3dad5c21a45658c963633b4d5a3643a767cd35e1 |
| Maobu Fetch_0.10.11_x64-setup.exe | 5314325 | 4ccc2d52f6071641fc2b9b18315d4d76a3b424f3230e8de08a34bc96ddf67d9d |
| maobu-fetch-extension-v0.10.11.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |
| extension.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |

五处版本仍同为 `0.10.11`，旧 identifier=`app.lumaget.desktop`。实际 PE 属性 ProductName=`Maobu Fetch`、ProductVersion/FileVersion=`0.10.11`、OriginalFilename=`maobu-fetch.exe`；MSI ProductVersion=`0.10.11`，File 表只有主程序，cabinet ID `Path` 正确对应 `maobu-fetch.exe`。两安装包均严格低于 30,000,000 字节，清单中无 `yt-dlp.exe`、`ffmpeg.exe`、`ffprobe.exe`。

7-Zip `26.02` 实际提取 NSIS **8 文件**（主程序、卸载程序、两张位图、四个插件 DLL），ParserWarnings 为空；MSI 提取 1 个主程序。两个扩展包各 39 文件，当前非测试源/manifest 与 dist 逐个 SHA 对齐，无测试脚本，双别名 ZIP 完全同字节。卸载程序只是静态提取，没有执行。

三份主程序长度均为 14,176,768 字节。严格验证完整唯一 Tauri 标记之后，只允许十进制偏移 **`12887152`** 的 3 字节 `UNK` / `MSI` / `NSS` 差异；两份归一后的整文件 SHA 均与便携 EXE 相同。原始值独立保留：

| Round5 主程序来源 | 未修改的实际 SHA-256 |
| --- | --- |
| 便携 EXE（UNK） | ace153d4075ae62838c48525f331d8fcc9af987469a17f3abde6b50c65b1851d |
| MSI 提取 EXE（MSI） | 3ac9ce123e7dd6b14f2818bb2d94b75cfe2fd519124373c7bcd415860f2a7320 |
| NSIS 提取 EXE（NSS） | b702c3aa663a3caa9d858d5f5870322e4a388d2172fa4bb0dde1fb5fbe9764aa |

实际程序和全部安装包/扩展提取文件的 workspace/profile 路径扫描命中 **0**，覆盖 UTF-8、UTF-16LE 偶/奇对齐及两种斜杠。主包 JavaScript 约 770 KB 的构建告警、Rust 90 条 warning（8 unused、82 dead code）仍由根任务保留为维护项，不因构建成功隐去。真实 release EXE `/v1/health`、NativeSink 提示显示/关闭、悬浮窗原生显示/置顶/焦点、安装/升级/回滚/卸载及 Windows 10 真机仍未验证；本轮未操作旧客户端、17433、个人数据库、设置、Cookie 或宿主机注册表。

## Round4 实际候选（阶段历史）

`pnpm tauri build` 的本轮 UTC 起点为 `2026-10-31T09:36:47.5111975Z`，根任务确认完整命令 exit 0、生成 NSIS/MSI 后，才于 `2026-10-31T09:42:02.0061181Z` 开始 fresh 验收：**13/13 通过，exit 0，STATIC_CANDIDATE_CHECKS_PASSED**。机器可读证据为 [round4 release-inspection.json](C:/Users/20269/AppData/Local/Temp/maobu-release-round4-804e9f1896d846cea1537c328700a634/release-inspection.json)，同目录保留五份实际产物、解包内容、只读 MSI 表、列表和 Get-FileHash 校验表。

本轮源码包含 `Store::open` 转入 `spawn_blocking`、JoinError 中文传播、GUI/CLI 启动失败显式退出码及脱敏提示。构建前根任务完整 round12 为 **1477 passed / 0 failed / 16 ignored**（98.48 秒），另 9 项 Range 回归全通过（5.06 秒）；279 份 Rust 源快照在测试结束时 Changed=0。该测试结果不替代下述实际产物验证，也不证明 NativeSink 弹窗已实际显示。

App/NSIS/MSI 的 UTC 修改时间分别为 `09:41:49.2977262Z`、`09:41:45.3123011Z`、`09:41:47.5340000Z`，均晚于本轮构建起点；没有使用旧候选。以下数值均读取本轮复制到 TEMP 的实际文件。

| Round4 阶段产物名称 | 实际字节 | Get-FileHash SHA-256 |
| --- | ---: | --- |
| maobu-fetch-v0.10.11-portable.exe | 14064128 | c1469b39327e1db65bc07d9d37dd27a1df0f31a4cb2ad261026e40e63d2ab4f8 |
| Maobu Fetch_0.10.11_x64_zh-CN.msi | 7032832 | 00e3118635158f1494929545cf28ef8d76b6e39b6055703d575ef36bcf1f4d0f |
| Maobu Fetch_0.10.11_x64-setup.exe | 5279241 | 03fad54f0d963f1a900c200a0f81db5ce03274fc05b62dfa7d0a876cfdf17b22 |
| maobu-fetch-extension-v0.10.11.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |
| extension.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |

五处版本均为 `0.10.11`，旧 identifier 仍为 `app.lumaget.desktop`；真实 ProductName=`Maobu Fetch`、ProductVersion/FileVersion=`0.10.11`、OriginalFilename=`maobu-fetch.exe`。MSI ProductVersion 为 `0.10.11`，File 表只有主程序，cabinet ID `Path` 正确对应文件。两安装包均严格低于 30,000,000 字节，没有三个媒体工具可执行文件。

本轮实际使用 7-Zip `26.02`：NSIS 提取 **8 个文件**，即主程序、卸载程序、两张位图和四个插件 DLL，ParserWarnings 为空；MSI 提取 1 个主程序。该事实与 round3 的 7 文件及 BadCmd 提示分别留档，不沿用旧轮结果，也不据此声称卸载程序已经运行。两个扩展包各 39 文件，当前非测试源/manifest 与 dist 逐文件 SHA 对齐，无测试脚本，两个别名 ZIP 完全同字节。

三份主程序长度均为 14,064,128 字节。完整唯一 Tauri 标记验证后，唯一允许的 bundle 类型差异在十进制偏移 `12777600`，严格仅归一 `UNK` / `MSI` / `NSS` 的 3 字节；两份归一后的整文件 SHA 均与便携 EXE 完全一致。未经修改的原始值如下：

| Round4 主程序来源 | 未修改的实际 SHA-256 |
| --- | --- |
| 便携 EXE（UNK） | c1469b39327e1db65bc07d9d37dd27a1df0f31a4cb2ad261026e40e63d2ab4f8 |
| MSI 提取 EXE（MSI） | 748ce23c0f053f205ca7e70a355b41f5012e45781875c45af5a01b1e80dc6608 |
| NSIS 提取 EXE（NSS） | 2dca785101e312ad23b729cfb00877a4d21d5a2ddaa11124b7575e8793064d03 |

App 和所有安装包/扩展提取文件的本机 workspace/profile 路径扫描命中 **0**，范围覆盖 UTF-8、UTF-16LE 偶/奇对齐及两种斜杠。只读 MSI 检查与解包不执行安装指令；真实 release EXE `/v1/health`、NativeSink 提示显示/关闭、安装/升级/回滚/卸载及 Windows 10 真机仍需单独验收。用户停止的桌面操作未恢复，本轮没有触碰旧客户端、17433、个人数据库、设置或 Cookie。

## Round3 实际候选（阶段历史）

`pnpm tauri build` 的本轮 UTC 起点为 `2026-10-31T08:20:23.6608125Z`，完整命令 exit 0。随后 `2026-10-31T08:25:11Z` 执行 fresh 验收：**13/13 通过，exit 0，STATIC_CANDIDATE_CHECKS_PASSED**。证据为 [round3 release-inspection.json](C:/Users/20269/AppData/Local/Temp/maobu-release-audit-0b762613dda74e93b9d17ea8ea2064f3/final-round3-4e3fcf87f45f4628a1dcbfe64ad93c81/release-inspection.json)，同目录保留全部实际产物、解包文件、包清单、只读 MSI 表与校验表。

源代码已包含后续 PikPak 空地址、原生 codec/工具回退和真实作品源选择修复。根任务最终 round10 **1471 passed / 0 failed / 16 ignored**，另 9 项 Range 回归全通过，274 个 Rust 源快照 Changed=0。App/NSIS/MSI 的 UTC 修改时间分别为 `08:24:29.4316084Z`、`08:24:25.6119548Z`、`08:24:27.7640000Z`，均晚于本轮起点，未复用旧文件。

| Round3 阶段产物名称 | 实际字节 | Get-FileHash SHA-256 |
| --- | ---: | --- |
| maobu-fetch-v0.10.11-portable.exe | 14058496 | 3606d4ed78b6851de210e2d64418a7d3f6c2d7debe554006f3acd8a86e749b21 |
| Maobu Fetch_0.10.11_x64_zh-CN.msi | 7032832 | 4564d66d6756d09894ec29cd410ed05a4633bef6cf3100c7f4e227dfc7596949 |
| Maobu Fetch_0.10.11_x64-setup.exe | 5281402 | 754f640776b52d3619d815367e9ff7acce36afe1df742306d8e7f0e348ef1452 |
| maobu-fetch-extension-v0.10.11.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |
| extension.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |

五处版本均为 `0.10.11`，旧 identifier 保留 `app.lumaget.desktop`；真实 ProductName=`Maobu Fetch`、ProductVersion/FileVersion=`0.10.11`、OriginalFilename=`maobu-fetch.exe`，MSI ProductVersion 同步。两安装包均严格低于 30,000,000 字节。NSIS 实际提取 7 个文件，MSI File 表及提取均只有主程序；不存在三个媒体工具可执行文件。两个扩展包各 39 个文件，全部当前非测试源/manifest 与 dist 按 SHA 对齐，没有测试脚本，双 ZIP 完全同字节。

三份 EXE 长度均为 14,058,496 字节，唯一 bundle 类型差异在十进制偏移 `12771680` 的 3 字节。严格验证完整唯一标记和对应 MSI/NSS 后，仅归一这 3 字节，两份整文件 SHA 都等于便携 EXE；原始值保留如下：

| Round3 主程序来源 | 未修改的实际 SHA-256 |
| --- | --- |
| 便携 EXE（UNK） | 3606d4ed78b6851de210e2d64418a7d3f6c2d7debe554006f3acd8a86e749b21 |
| MSI 提取 EXE（MSI） | 703547c27e080af4b821b78a28ed07e92cffe99ec345c2eea0300e326c4a8898 |
| NSIS 提取 EXE（NSS） | 92b40727db430962adfcbc3fc90a6248a5754169a7ccae322fab9bf77cc450e8 |

App、所有安装包/扩展提取文件的实际 workspace/profile 路径扫描命中 **0**：UTF-8、UTF-16LE 两种字节对齐及两种斜杠写法均检查。NSIS 的 `BadCmd=13` 解析提示仍原样记录；包内主程序严格内容校验通过，不把静态解包当成安装指令运行验证。没有启动 EXE 或安装器，没有发布资产；安装生命周期和 release `/v1/health` 的未验证边界保留在本文末尾。

## 旧构建留档

构建前已将原 EXE、NSIS、MSI 复制到明确创建的独立 TEMP：`maobu-release-audit-0b762613dda74e93b9d17ea8ea2064f3/baseline`。同目录的 `baseline-manifest.json` 标为 `OLD_BASELINE_ONLY`，保存源路径、UTC 修改时间、大小和实际 `Get-FileHash -Algorithm SHA256`。

这些旧文件仅供回归留档，绝不作为当前修改后的构建结果。旧 EXE 实测存在 UTF-8 与 UTF-16 的本机 Cargo 源路径，并且 OriginalFilename 为空；它们是发现并需要新构建验证的旧缺口。

## REL2：原生开发路径残留

旧 AWS-LC 构建输出实际采用 CC builder 和 MSVC `14.44.35207/Hostx64/x64/cl.exe`，没有使用 Clang。Rust 的 `strip=true` 不能移除 `file!()` / C `__FILE__` 已成为程序数据的字符串；[Rust 官方路径映射说明](https://doc.rust-lang.org/rustc/remap-source-paths.html)也明确区分编译器自身和外部工具引入的路径。

使用同一台机器的实际 cl 编译器完成独立小样本：中文、空格源文件与 header，导出窄字符 `__FILE__` 和宽字符展开，链接成无 CRT 的 PE，仅静态读取其字节。未映射的对照 PE 两种编码均有真实路径；`/experimental:deterministic` 配合 `/pathmap:C:\Users\20269\=/build-user/` 后，C 与 C++ 两种语言的 UTF-8、UTF-16LE 均不再含真实主目录，均含逻辑映射前缀且保留中文文件名。重复 `/pathmap` 累积映射也实际验证通过。

没有采用被忽略的选项：`/FC-` 在该编译器实际报 D8021；单独 `/pathmap` 报 D9007 并忽略，必须使用已验证组合。早期直接扫描 COFF object 会把文件符号与运行时字符串混淆，最终判定使用链接后的导出数组 PE。

`scripts/release-nativeflags.mjs` 仅给 release 子进程添加目标级 CFLAGS/CXXFLAGS；已有通用和目标参数保留，不修改系统环境、不关闭错误检查、不修改第三方安全源码。`scripts/tauri.mjs` 每次 release build 先执行同编译器的 C/C++ 宽窄对照预检，再开始构建；未知/被忽略选项、未验收自定义编译器或不安全的参数解析均明确失败。

实际执行 `node scripts/release-rustflags.test.mjs`：**exit 0**，包含原 Rust 中文/空格 `file!()` 的真实编译运行，以及上述 MSVC C/C++ 两种编码的 PE 验证。新候选仍必须再次扫描，不能用此小样本代替实际产物。

## 可复用验收脚本

`scripts/audit/inspect-release.ps1` 读取候选目录，并要求本轮 build 的 UTC 起点；拒绝修改时间早于该起点的旧 EXE/安装包。所有拷贝、解包、便携 EXE、版本化扩展 ZIP、兼容 `extension.zip`、清单和校验表只写入新建 TEMP 目录。

检查包括五处版本与旧标识、真实 PE ProductName/ProductVersion/FileVersion/OriginalFilename、NSIS/MSI 严格小于 30,000,000 字节、7-Zip 文件列表、MSI 只读 File/Property 表、包内提取 EXE 与本轮 EXE 的严格代码内容一致、媒体工具排除、当前扩展源文件与 dist 一致、双 ZIP 完全同字节、实际 Get-FileHash，以及程序/解包内容中的 UTF-8 与两种字节对齐的 UTF-16LE 本机路径。

中间构建的三份 EXE 原始哈希不同，实测全部差异仅为同一偏移的 3 字节 `UNK` / `MSI` / `NSS`。这来自实际依赖 [tauri-utils 2.8.3 的 platform.rs](C:/Users/20269/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-utils-2.8.3/src/platform.rs:344)：344—349 行注释及静态变量明确在 build 中二进制修改 `__TAURI_BUNDLE_TYPE_VAR_UNK`，359—360 行识别 MSI 与 NSS。验收要求便携 EXE 中完整 UNK 标记唯一、包内同一偏移为对应 MSI/NSS；仅将这 3 字节归一，之后整文件 SHA-256 必须完全一致。其他任何字节变化仍拒绝；原始三份 SHA 独立保留，不能宽泛替换其他 UNK 字样。

MSI 通过 WindowsInstaller COM 的只读 mode 0 打开数据库，不执行安装或 custom action。旧 MSI 的 cabinet 文件 ID 为 `Path`，已实测借 File 表正确映射到 `maobu-fetch.exe`，不会因内部别名而漏检。NSIS 的 7-Zip 解析可能给出 BadCmd 提示，脚本保留该提示并记录生成 `.nsi` 的 File 指令；提取 EXE 的实际哈希仍必须与候选一致，但不据此声称安装指令完整运行过。

脚本及三个小模块已实际通过 PowerShell 解析；`powershell -NoProfile -File scripts/audit/release/helpers.test.ps1` 最终 **12/12 通过**，覆盖 UTF-8、奇对齐 UTF-16、正斜杠路径、旧构建时间拒绝、媒体工具名、ZIP 路径穿越在提取前拒绝、无 BOM 中文 JSON 明确按 UTF-8 读取、两种合法包标记，以及同长其他位置一字节篡改/错误包标记/重复完整 UNK 标记均拒绝。MSI COM 空返回显式丢弃，避免污染清单。旧 baseline 的只读 MSI/7-Zip/helper 练习保持 `OLD_BASELINE_HELPER_TEST_ONLY`，没有当作候选验收。

## 构建分轮边界

首轮 `pnpm tauri build`（UTC 起点 `2026-10-31T07:16:45.8594796Z`）成功后又有设置兼容修复，因此只作为 intermediate。该中间产物真实静态预检 **13/13 通过**，其哈希没有填入最终表。

设置修复后的 release round2 起点为 `2026-10-31T07:28:41.1905127Z`，根任务确认完整命令 exit 0 后，才另建全新 TEMP 目录进行最终验收。最终 App/NSIS/MSI 修改时间分别为 `07:34:31.5592345Z`、`07:34:27.9498000Z`、`07:34:29.9040000Z`，均晚于本轮起点；没有拿旧 baseline 或 intermediate 代替。

## Round2 候选实际结果（历史）

`2026-10-31T07:35:21Z` 实际执行 `inspect-release.ps1`：**13/13 通过，exit 0，STATIC_CANDIDATE_CHECKS_PASSED**。完整机器可读证据：[release-inspection.json](C:/Users/20269/AppData/Local/Temp/maobu-release-audit-0b762613dda74e93b9d17ea8ea2064f3/final-candidate-6ae15c2ec36d477fb76baf5e2dbac98c/release-inspection.json)，同目录保留 7-Zip 列表、提取内容、MSI 只读清单、实际校验表及五份 TEMP 产物。未发布任何资产。

五处版本均为 `0.10.11`，旧 identifier 仍为 `app.lumaget.desktop`。真实 PE 属性：ProductName=`Maobu Fetch`，ProductVersion/FileVersion=`0.10.11`，OriginalFilename=`maobu-fetch.exe`。MSI ProductVersion 同样为 `0.10.11`，File 表中只有目标主程序，内部 cabinet ID `Path` 正确对应 `maobu-fetch.exe`。

| Round2 历史产物名称 | 实际字节 | Get-FileHash SHA-256 |
| --- | ---: | --- |
| maobu-fetch-v0.10.11-portable.exe | 14045696 | aaf65d9ddba15795c486257650cae3d8e52b0c05aebc534e5106f924e4a3c63f |
| Maobu Fetch_0.10.11_x64_zh-CN.msi | 7024640 | 803cd21b38704b232af8f14c2b63d4f3430a9834240b2c98b1c5b433ec702802 |
| Maobu Fetch_0.10.11_x64-setup.exe | 5277073 | adbd3366ee22c3880878527a14408757cd675332885b6a3c00f12cb66d2ce77c |
| maobu-fetch-extension-v0.10.11.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |
| extension.zip | 116500 | 64b3675f6e0be06cd79b135e3455dc28ab2136c5552de46af9e3ca9ff8b45a8c |

NSIS/MSI 均严格低于 30,000,000 字节。NSIS 实际提取 7 个文件：主程序、两张安装界面位图和四个 NSIS 插件 DLL；MSI File 表及实际 cabinet 提取均只有主程序。没有 `yt-dlp.exe`、`ffmpeg.exe` 或 `ffprobe.exe`。两个扩展包各有 39 个文件，当前非测试源文件/manifest 与 dist 逐个 SHA 对齐，没有测试脚本，双命名 ZIP 完全同字节。

最终三份主程序长度均为 14,045,696 字节。两安装类型的唯一 3 字节差异位于十进制偏移 `12761648`，对应前述完整 Tauri 标记；仅归一这 3 字节后，两份整文件 SHA 都等于便携 EXE 的 `aaf65d9ddba15795c486257650cae3d8e52b0c05aebc534e5106f924e4a3c63f`。原始实际值分别是：

| 主程序来源 | 未修改的实际 SHA-256 |
| --- | --- |
| 便携 EXE（UNK） | aaf65d9ddba15795c486257650cae3d8e52b0c05aebc534e5106f924e4a3c63f |
| MSI 提取 EXE（MSI） | 820e98f7c3fce7c8baae15886e6c2b7c88c1ac6950fc3bb7536a61673098c7da |
| NSIS 提取 EXE（NSS） | 9ad9301a4262da133e029b22e1636ec351d7ac0e9c4d156228943ff81bf1fac8 |

App、两安装包提取内容及双扩展包提取文件中，实际 workspace/profile 路径的 UTF-8、UTF-16LE 偶对齐和奇对齐扫描命中均为 **0**，两种斜杠写法均已检查。这是本轮候选的真实结果，REL2 不再只依赖编译小样本。扫描范围是本机实际开发路径，不声称证明所有任意文本不存在。

NSIS 的 7-Zip 解析仍报告 `NSIS-3 Unicode BadCmd=13`，已原样保存在 JSON 与文件清单；实际主程序可提取且严格代码校验通过，生成 `.nsi` 的三条条件 File 指令也已记录。该静态解包证据不能代替安装指令运行验证。

## 未验证边界

机器存在 Hyper-V 模块/命令，但 `Get-VM` 实际权限拒绝，Hyper-V 不可访问；没有已准备且可用的隔离 VM，也没有可用 Windows Sandbox、VirtualBox、VMware 或 QEMU。未冒险在主机安装、覆盖升级、回滚或卸载，相关安装生命周期仍未验证。真实 release EXE 的 `/v1/health` 启动检查也不属于本静态脚本，不能用原生单测 AppHandle 或旧 EXE 结果替代。
