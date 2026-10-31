# 极简透明下载进度悬浮窗：实现与验证

最新数字、轨道与状态排版已由[显示细调专项](2026-11-01-progress-window-polish.md)接续，当前包为release7。本篇保留紧凑透明基础实现与release6阶段证据。

修订日期：2026-11-01（Asia/Shanghai）。本文件沿用2026-10-31审计系列命名。项目版本0.10.11；本轮只收窄已有进度悬浮窗的视觉、尺寸和交互，不修改下载协议或用户文件。

**当前结果：极简方案的源码、前端检查、21组浏览器检查、full15 Rust回归及release6构建/13项静态产物检查均已通过。原生Windows透明、置顶、焦点及多屏行为未实机验收，不能由浏览器截图或静态产物检查替代。**

## 用户看到的变化

- 常态只保留**224×28逻辑像素**的细进度条、小任务数和可确认的百分比，没有常驻标题、文件列表或操作栏。
- 鼠标悬停或键盘聚焦后，详情从细条上方展开。细条仍固定在原来的屏幕位置，避免窗口向上变大后触发区域离开鼠标造成反复开合。
- 详情默认显示最多3条任务，为**224×190**；选择更多后最多显示6条，为**224×298**。更多任务进入主窗口查看。单任务和较小工作区按真实数量及可用空间收缩。
- 文件名、当前速度、暂停、查看任务、置顶切换和隐藏按钮只在详情出现。取消置顶仍保存到已有设置；暂停后的任务可从主窗口继续。
- 鼠标离开且内部没有键盘焦点时，等待150毫秒后收起。焦点在内部控件之间移动时保留详情；窗口失焦会清除残留焦点/悬停意图。
- Esc第一次收起详情，第二次隐藏当前批次。关闭按钮只隐藏浮窗，沿用原有批次隐藏规则，不取消下载。

以上尺寸是100%缩放的逻辑尺寸。后端依据当前屏幕工作区、缩放和边界计算实际物理矩形，包含任务栏、负坐标屏幕和极小工作区的布局测试。**本轮没有实际移动Windows原生窗口来验证这些结果。**

## 汇总进度与兼容

紧凑条不从最多6条预览记录计算整个队列，也不使用各任务百分比的简单平均。后端遍历全部当前活动任务，以`u128`有检查加法累计文件总字节及`min(downloaded,total)`，再按字节加权计算汇总百分比。

任一任务总长未知、仍在获取磁力元数据，或队列为空时，汇总为`null`；溢出也保持未知。前端显示静态未知轨道和破折号，没有伪造百分比或持续跑动的模拟填充。所有下载字节、速度和状态仍来自真实Manager事件；完成后的BT做种不混入活动下载。

沿用事件`download-progress-updated`及原有7个命令，新增第8个命令`progress_window_set_revealed(revealed)`，返回同样的安全Snapshot。新字段如下：

| 字段 | 含义及默认行为 |
| --- | --- |
| `revealed` | 本次会话的详情显隐状态，默认false；不写入用户设置。空队列、禁用或隐藏批次后重置，迟到的hover请求不能重开不活动批次。 |
| `summary_percent` | 后端全量队列计算的number或null。前端旧缓存缺字段时按null处理，不从预览行补算。 |

前端将新字段声明为可选，缺少`revealed`按false读取。没有新增数据库结构或迁移，没有改变17433、X-Luma头、HMAC或现有命令名。Snapshot继续排除URL、下载路径、Cookie和认证头；新命令也限制为主窗口及进度窗口调用。

## 显隐队列与透明范围

hover显隐使用独立的串行意图队列，不经过任务/置顶操作的busy互斥。显示请求尚未返回时鼠标已经离开，队列仍会补发最终收起意图；后端拒绝旧批次的显示请求不会引发无限重试。失败仅提示通用可操作信息，不回显可能带凭据的原始错误；窗口卸载后忽略迟到响应。普通快照仍用revision拒绝迟到的旧数据。

详情节点在收起时真正卸载，没有用`opacity:0`保留不可见可聚焦控件。后端同时缩小实际HWND尺寸，避免用大块透明画布长期覆盖其他应用的点击区域。显示、隐藏、尺寸和置顶仍走已有统一Win32无激活路径，Webview创建继续在后台线程执行。

透明分为三层：

1. 后端窗口启用`transparent(true)`并设置窗口/Webview透明背景。
2. 正式`index.html`在主脚本之前，仅对`view=download-progress`同步加页面class和局部透明样式；全局浅/深色底色不能把进度页重新填成不透明矩形。
3. 进度页的html、body、root及常态容器透明；只有展开详情有克制的半透明衬底和细边。没有新增玻璃光斑、彩色渐变或大面积装饰，也没有改变主窗口及播放器的背景。

初始Snapshot读取失败时，未知状态仍允许用户悬停/聚焦请求详情，由后端判定实际活动状态；详情提供错误重试。只有已经确认的禁用、空队列等状态才清理显隐意图，不会把重试入口锁在收起的细条里。

## 已执行验证

| 检查 | 实际结果 | 证据与边界 |
| --- | --- | --- |
| 前端基础检查 | `pnpm run check`、`pnpm run build`、`pnpm run extension:build`通过 | 根任务日志`%TEMP%/maobu-compact-check.log`、`maobu-compact-build.log`、`maobu-compact-extension.log`。构建仍提示约774kB主脚本体积债务，没有关闭警告掩盖。 |
| 展示/显隐队列规则 | **27项通过**；i18n **19/19** | 包括旧字段默认、真实汇总字段、未知进度、快速进退、失败恢复、拒绝后不循环及卸载迟到。已纳入check。 |
| IPC与扩展 | **162个命令、161个实际调用契约**无问题；扩展**193通过、0失败** | 捕获实际API载荷并核对注册、参数与Snapshot返回类型；不是原生窗口运行证明。 |
| 后端进度专项 | **14/14通过**，0.02秒 | 包括全量加权、未知元数据、溢出、暂态revealed及非活动批次、真实尺寸算法、凭据排除、增量状态和后台创建线程/JoinError。测试中的synthetic worker panic是验证JoinError传播的预期夹具，最终用例通过，不是实际客户端崩溃。 |
| 当前浏览器视图 | **21组通过、18张截图，errors=0** | 实际React、隔离API替身，原生尺寸用250毫秒延迟模拟；1280×720/1440×900、100%/125%、浅/深色及500任务输入。覆盖初始错误重试、键盘焦点、Esc、忙碌操作期间离开、3/6条展开、透明首帧及主窗口背景隔离。 |
| full15 Rust整套 | **1497通过、0失败、16忽略**，114.58秒；Range另**9通过、0失败**，5.06秒 | 根任务`%TEMP%/maobu-final-cargo-round15.log`与独立退出码JSON；293份Rust源码快照Changed=0。忽略项不算成功，专项计数不再累加到1497。 |
| release6与最新安装包 | **构建实际exit 0，fresh静态13/13通过** | 构建起点UTC`2026-10-31T16:21:14.7586040Z`，退出码独立落盘；新体积、SHA及包内清单见[发布专项](2026-10-31-release-inspection.md)，没有沿用release5数值。293份Rust及115份前端源码快照Changed=0。 |

当前浏览器证据为[21组结果JSON](C:/Users/20269/AppData/Local/Temp/maobu-compact-ui-uiHGGo/results.json)，同目录保留18张截图；日志`%TEMP%/maobu-compact-progress-ui-final.log`。此前`maobu-compact-ui-bg9ULF`的20组/18图保留为历史，不代替本次21组结果。

当前[细条预览PNG](C:/Users/20269/AppData/Local/Temp/maobu-compact-ui-uiHGGo/compact-preview.png)为224×28，alpha范围0～255，其中4961/6272像素完全透明，约79%。这是浏览器导出PNG的像素事实，**不是Windows原生桌面穿透合成或点击穿透测试**。根任务已经目检常态、详情与深色截图。

后端专项日志：[progress-tests.log](C:/Users/20269/AppData/Local/Temp/maobu-compact-backend-bb3bd63bbe824b9693f41761fe1c687f/progress-tests.log)。完整退出码：[cargo-result.json](C:/Users/20269/AppData/Local/Temp/maobu-final-round15-b5773ea817bc4c7baf806221a48489e8/cargo-result.json)；同目录保留Rust/前端源码快照。

最终构建退出码：[release-round6-result.json](C:/Users/20269/AppData/Local/Temp/maobu-final-round15-b5773ea817bc4c7baf806221a48489e8/release-round6-result.json)；全新静态检查：[release-inspection.json](C:/Users/20269/AppData/Local/Temp/maobu-release-round6-27a39dd6686046d3990adef91004e332/release-inspection.json)。本次13项通过只覆盖构建产物，不表示原生窗口或安装器已经执行。

## 仍未验证与交付范围

用户此前按Esc停止的桌面控制尚未恢复，本轮没有启动可见原生窗口、关闭现有客户端或占用17433。原生透明合成、置顶取消/恢复、实际焦点是否保持、多屏/DPI切换和桌面工作区最终位置仍未验；CSS透明、PNG alpha、模拟延迟resize及纯布局测试均不能替代这些项目。

本轮没有执行安装器或发布资产。release6构建/包内检查、原生启动与安装生命周期以[发布专项](2026-10-31-release-inspection.md)及[整改台账](2026-10-31-remediation-status.md)为准。旧版列表式浮窗记录保留在[前版专项](2026-10-31-progress-window.md)，其中340px大窗的验证不能当成本次极简方案结果。

## 代码索引

- [透明首帧入口](../../index.html)、[视图](../../src/components/progress/DownloadProgressView.tsx)、[细条](../../src/components/progress/ProgressSummary.tsx)、[详情](../../src/components/progress/ProgressDetails.tsx)、[局部透明样式](../../src/components/progress/progress-window.css)。
- [显隐交互](../../src/components/progress/useProgressReveal.ts)、[串行意图队列](../../src/components/progress/reveal-queue.ts)、[前端规则与回归](../../src/components/progress/progress-model.test.ts)、[API/DTO](../../src/components/progress/progress-api.ts)。
- [后端汇总](../../src-tauri/src/progress_window/summary.rs)、[布局](../../src-tauri/src/progress_window/layout.rs)、[状态](../../src-tauri/src/progress_window/state.rs)、[命令](../../src-tauri/src/progress_window/commands.rs)、[后台创建](../../src-tauri/src/progress_window/creation.rs)。
