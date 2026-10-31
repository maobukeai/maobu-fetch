# 下载进度悬浮窗：实现与分层验收

**历史设计说明：本文的大面板与full14/release5为已完成的阶段记录。用户后续要求极简透明，当前常态已改为224×28细条、悬停后显示详情；最新实现、full15与release6证据以[极简透明浮窗专项](2026-10-31-compact-progress-window.md)为准，旧截图不代表当前默认外形。**

日期：2026-10-31（Asia/Shanghai）。项目版本：0.10.11。

本专项对应用户新增的右下角下载进度窗需求。**包含本功能及后台创建补强的完整 Rust round14 已实际通过：1494 通过、0 失败、16 忽略，另 Range 9 项通过；最新核定 release round5 构建实际 exit 0，全新静态检查 13/13 通过。** 历史 round13 曾发生原生聚合测试进程栈溢出，失败记录保留，不因后续通过而删除，也不把它与没有在该聚合注册的悬浮窗混作同一原因。既有 round12/release round4 不包含本功能，不能替代当前门禁。原生 OS 行为仍未验，最新统一状态见[整改台账](2026-10-31-remediation-status.md)及[发布专项](2026-10-31-release-inspection.md)。

## 用户可见行为

- 有进行中任务时，在屏幕工作区右下角显示独立、紧凑的进度窗口；主窗口最小化后仍可查看。
- 默认最多显示 3 条；展开最多 6 条，更多任务显示真实剩余数量，并提供打开主窗口的入口。后台只发送最多 6 行，500 条队列也不会把完整历史表送入小窗。
- 默认置顶，支持一键取消；选择持久保存，重启后保留。设置页可关闭进度悬浮窗，也可手动重新显示被关闭的同批任务。
- 展示真实文件名、状态、已下载/总字节、速度及可用的剩余时间；未知大小不显示虚构百分比。磁力元数据未取得时显示“待获取”，不借用猜测的文件名或大小。
- 可暂停允许暂停的任务，或打开主窗口并定位任务；校验期间不提供不受支持的暂停。暂停后该行退出进行中列表，继续下载在主窗口操作。
- 关闭或按 Esc 只隐藏悬浮窗，不取消下载。同一批任务继续更新不会立刻重开；新增进行中任务、原批次结束后的下一批，或手动恢复可以重新显示。

本功能固定锚定工作区右下角，未提供可拖拽标题栏。完成提醒仍使用原有 30 秒/可配置提醒流程，不把完成后的 BT 做种当作活跃下载显示。

## 状态、事件与安全契约

窗口 label 为 `download-progress`，加载本地 `index.html?view=download-progress`。后端先读取一次现有任务，随后监听既有 `task-created`、`task-updated`、`task-removed` 和设置变化，维护内存中的安全投影；没有每次进度事件查询完整 SQLite 历史，也没有常驻高频轮询。

事件驱动更新通过单次 250 ms 等待合并突发事件。初始读取期间到达的新增、更新与删除先记录，合并初始结果时优先应用新事件，防止旧初始结果复活已删除行。前端同时按单调 revision 拒绝倒退快照。

仅 `Queued`、`Downloading`、`Verifying`、`WaitingNetwork` 进入列表；暂停、计划、完成、失败、取消等状态不计入。进行中的下载/校验优先，其后按优先级、队列位置、创建时间和 ID 稳定排序。总数量与聚合速度统计全部活跃任务，显示行最多 6 条；排队/校验等非下载状态不借用旧速度。

专用 `download-progress-updated` 事件只定向发送给悬浮窗，与 `progress_window_snapshot` 使用同一 DTO：

| 字段组 | 内容 |
| --- | --- |
| 行数据 | `id`、脱敏文件名、真实状态、已下载/总字节、`speed_bps`、可用的 `eta_seconds`、元数据阶段、允许的动作 |
| 汇总 | `revision`、真实 `total_count`、`total_speed_bps` |
| 交互与外观 | 开关、置顶、展开、加载状态、主题/颜色方案、语言、强调色、脱敏错误 |

该 DTO 不包含源 URL、签名地址、目标路径、Cookie、Authorization、代理密码、媒体直链或完整 `DownloadTask`/`AppSettings`。置顶补丁向主窗口单独发送设置变化，悬浮窗收到的仍是限制后的安全快照。

7 个 IPC 命令分别为读取快照、切换置顶、切换展开、暂停任务、打开主窗口、隐藏及恢复。它们均校验实际调用窗口只允许 `main` 或 `download-progress`，均返回 `Result<Snapshot, String>`。暂停还会重新读取当前任务，拒绝不存在或已不支持该动作的状态，再调用正式 Manager。

审阅曾捕获 `show_main` 返回 void、而前端按 Snapshot 消费的实际契约缺口；已改为返回安全快照。根任务的 IPC 审计补入返回 DTO 核对，未通过修改界面夹具或忽略 undefined 掩盖问题。

## 设置兼容与失败处理

新增 `progress_window_enabled` 与 `progress_window_always_on_top` 两个布尔设置，旧 JSON 缺失时均安全默认 `true`，显式 `false` 在往返序列化后保持。使用现有设置 JSON 存储，没有重建表、删除任务或修改既有下载文件。

置顶操作只修改当前设置中的对应字段，不用悬浮窗持有的旧完整设置覆盖下载目录、代理、凭据或其他选项。设置操作锁与整体保存、备份恢复共用，防止 SQLite 提交与内存刷新交错；显式保存整个设置快照仍按正常先后执行语义处理，本功能不宣称实现全设置冲突合并。

持久化失败时不提交内存设置；命令中包含恢复原生置顶状态的路径，恢复失败也返回明确错误。进行中的置顶保存会阻止合并渲染重新应用旧置顶值。**真实 SQLite 失败保留已测试；原生窗口回滚路径尚未实机执行。** 后台窗口创建/更新错误会脱敏并通知主窗口；初次任务读取失败可由恢复入口手动重试。

## Windows 原生窗口策略与未验证边界

后端使用锁定 Tauri 2.11.5 的 `Monitor::work_area()`，按物理坐标计算位置，考虑任务栏工作区、缩放及负坐标屏幕；宽度目标为 340 逻辑像素，高度为 `74 + 64 × 可见行数`，并夹紧到当前工作区。

只设置 `.focused(false)` 不足以证明后续显示不会激活窗口：本地锁定 Tao 0.35.3 的 `window_state.rs::apply_diff` 在可见状态变化或部分样式变化时可能调用 `SW_SHOW`。因此 Windows 路径统一由本功能的 OS 小模块负责：

- 创建时隐藏且不聚焦，保持可获得键盘焦点；不永久禁用 focusable。
- 显示使用 `SW_SHOWNOACTIVATE`，隐藏使用 `SW_HIDE`，并读取 `IsWindowVisible` 核对真实状态；已有目标状态时不重复显示。
- 置顶与尺寸/定位使用 `SetWindowPos` 和 `SWP_NOACTIVATE`；取消置顶后不在每次进度更新中重复改变层级，相同矩形不重复调整。
- 该 HWND 后续不混用 Tao 的 `show/hide/set_always_on_top/set_size`，避免原生可见状态与 Tao 的缓存路径交错；新窗口能力也未开放这些前端原始操作。
- `ScaleFactorChanged` / `Moved` 触发重新读取工作区并锚定；无边框窗口关闭原生 shadow，避免尺寸计算借用隐藏边距。

以上是**实现与本地依赖源码核对证据**，不是 Windows 前台焦点或多屏行为的运行证明。用户此前按 Esc 停止桌面控制，当前尚未恢复，因此没有显示真实进度窗口、切换真实前台程序、改变系统 DPI 或运行新客户端验证。原生置顶/取消置顶、显示不抢焦点、Alt+F4/退出、多屏与任务栏变更仍须独立验收；浏览器截图不能代替这些项目。

### Windows WebView2 创建线程补查

后续源码复核发现旧实现仍在 `run_on_main_thread` 的闭包内间接调用 `WebviewWindowBuilder::build`。锁定 Tauri 2.11.5 的 `webview/webview_window.rs` 第 56～59 行明确记录 Windows 在同步命令或事件处理器中创建 Webview 的死锁限制，要求异步命令/独立线程；该调用链已实际修正，没有仅以“原生未验”留下已知错误路径。

当前分为两个阶段：单一、串行的事件 worker 先 await `creation::prepare`，其 `spawn_blocking` 在线程池中创建隐藏且不聚焦的窗口；创建返回后，才向主事件线程投递 Win32 定位、置顶、显示/隐藏及安全快照发送。`native::apply` 已不包含 builder 或创建调用。准备阶段前后重新检查任务/窗口状态，创建过程中用户隐藏或关闭开关也不会强行显示；如果新任务恰好在准备检查之后到达，UI 阶段仅通知下一轮准备，不在事件线程临时创建或误报失败。

补充线程派发回归使用合成创建闭包验证真实 `spawn_blocking` 运行在线程池、错误原样传播、worker panic 以 JoinError 返回而不让调用方 panic；它不创建真实 Webview，也不能替代后续原生首次创建/重新显示验收。补强后的 compile4 与 11+2 项专项均已实际通过，旧 compile3/9+2 证据保留历史。

## 已返回的实际检查

| 检查层 | 结果 | 能证明与不能证明的范围 |
| --- | --- | --- |
| Rust 模块编译 | compile4 exit 0，28.72 秒 | 使用实际集成模块及 7 个命令，包含 Snapshot 返回类型修复和后台创建线程补强；不等于完整 Rust 套件通过。此前 compile3 exit 0/24.66 秒为线程补强之前的历史。 |
| Rust 逻辑专项 | **11/11 通过**，0.02 秒 | 活跃状态/做种排除、真实值与安全投影、未知磁力元数据、初始事件竞态、批次隐藏、500 任务边界、旧设置兼容、工作区算法、窗口权限，以及新 2 项真实线程派发/错误/JoinError 回归。此前 9/9 为历史，不漏计新增线程专项。 |
| 实际 SQLite 专项 | **2/2 通过**，0.04 秒 | 只改置顶且保留其他设置、重开后读取；注入写入失败时内存及原数据库设置均保留。没有创建原生窗口。 |
| TypeScript 规则 | **13 项通过** | 百分比未知值/边界、行数、revision 选择等规则，不是 OS 窗口行为。 |
| 文案与基础检查 | i18n **19/19**；`pnpm run check` exit 0，含扩展 **193/193**、IPC **161 commands / 160 calls** | 文案、类型、已有扩展及新增 IPC 契约；不据此声明原生定位、置顶或下载全链路通过。 |
| 实际 React 浏览器检查（最终） | **20 组通过、9 张截图**；浅/深色及新增展开图均已由根任务目检 | 初始/错误/空态、真实数值显示、未知大小、展开、键盘、忙碌/重复操作、失败反馈、迟到快照；1280×720 / 1440×900、100% / 125%、浅/深色及 500 条输入；另含未保存设置不得恢复、已保存后可恢复及完整 6 行高度。API/OS 为替身。旧 17 组/8 图为历史轮次，不代替最终结果。 |
| 完整 Rust round13（历史失败） | 原生聚合进程退出 `STATUS_STACK_OVERFLOW / 0xc00000fd`，无最终通过统计 | 日志 `%TEMP%/maobu-final-cargo-round13.log`；不算通过，不以调大栈或删除场景隐藏失败。 |
| 完整 Rust round14 | **1494 通过、0 失败、16 忽略**，113.34 秒；另 Range **9 通过、0 失败**，5.06 秒 | 默认原生聚合所有阶段、真实外部媒体工具、临时 Chrome/Edge 桥均执行完成；292 份 Rust 源码快照 Changed=0。日志 `%TEMP%/maobu-final-cargo-round14.log`；忽略项不计成功，不能据此声称原生悬浮窗已经运行。 |
| release round5（最新核定） | **构建 exit 0，全新静态 13/13 通过** | UTC 起点 `2026-10-31T15:13:30.9942952Z`；NSIS 5,314,325B、MSI 7,086,080B、App 14,176,768B。新产物实际 SHA、文件清单与资源核对见[发布专项](2026-10-31-release-inspection.md)，不沿用旧 round4 或失效会话的产物；未运行原生窗口或安装器。 |

Rust 实际证据位于 `%TEMP%/maobu-progress-backend-0b574a9b446a42d3bd70ef1cbb66b0bb`，当前日志为 `progress-tests-compile4.log`、`settings-tests-compile4.log`；编译日志为 `%TEMP%/maobu-progress-backend-compile4.log`。该目录当前复制测试 EXE `progress-tests-compile4.exe` 的 SHA-256 为 `b16f9035efd785103ac55cfaa255b4010357a87048469b4541df3192e785df44`。compile3 日志与 EXE 同目录保留为历史，不能代替线程补强之后的证据。

加入根任务授权的聚合诊断入口后，compile5 再次 exit 0（20.51 秒），同一进度源码的 **11/11 与 SQLite 2/2** 再次通过（分别 0.02 / 0.04 秒）。日志为同目录 `progress-tests-compile5.log`、`settings-tests-compile5.log`；编译日志 `%TEMP%/maobu-progress-backend-compile5.log`。复制的 `progress-tests-compile5.exe` SHA-256 为 `6ceb2a45b02ab0ddba42ec448da15e07b8c7d15c2fab0d85a4589843d0e4cff1`，提供给根任务及聚合诊断 agent 复用；它仍不等于完整套件通过。

round13 后的仅测量诊断没有 poll 任何场景，原型 future 实际大小为 **26,280 字节**，不是只测 Box 指针的 8 字节。该数值不足以证明 2 MB 测试线程栈不足；入口 Box 化是单变量实验，round14 通过也不能据此断言已找到历史栈溢出的唯一根因。根任务核对 compile5 测试 EXE 的实际 SHA 后，在隔离、真实外部媒体工具及临时浏览器桥均启用的条件下，另一次精确默认聚合复跑已 **exit 0、1/1 通过，109.91 秒**，日志 `%TEMP%/maobu-native-aggregate-repeat.log` 的所有阶段均到达 done。两次当前结果为已观测稳定性证据，未修改线程栈或删除场景；仍不等于唯一根因证明，也不构成真实悬浮窗 OS 行为验收。最新核定 release round5 构建及静态检查的通过范围单列于上表。

最终实际 React 检查证据：[results.json](C:/Users/20269/AppData/Local/Temp/maobu-progress-ui-FC6frk/results.json)，同目录保留 9 张截图，日志为 `%TEMP%/maobu-progress-ui-final.log`。其 scope 明确说明 API/OS 行为替代，不冒充 Native 验收。此前 `maobu-progress-ui-qKy2l9` 的 17 组/8 图保留为历史证据。

## 代码索引

- [前端视图](../../src/components/progress/DownloadProgressView.tsx)、[事件订阅与异步状态](../../src/components/progress/useDownloadProgress.ts)、[安全 DTO/IPC](../../src/components/progress/progress-api.ts)、[展示规则](../../src/components/progress/progress-model.ts)。
- [后端入口](../../src-tauri/src/progress_window.rs)、[安全缓存](../../src-tauri/src/progress_window/state.rs)、[事件合并](../../src-tauri/src/progress_window/runtime.rs)、[IPC 命令](../../src-tauri/src/progress_window/commands.rs)。
- [后台创建](../../src-tauri/src/progress_window/creation.rs)、[工作区算法](../../src-tauri/src/progress_window/layout.rs)、[窗口生命周期](../../src-tauri/src/progress_window/native.rs)、[Windows OS 操作](../../src-tauri/src/progress_window/os.rs)、[置顶设置补丁](../../src-tauri/src/manager/progress_settings.rs)、[专项测试](../../src-tauri/src/progress_window/tests.rs)。

本专项没有执行安装器或发布产物，没有关闭用户旧客户端、接管固定 17433 或修改真实用户任务；源码与文档的最终统一门禁仍由根任务记录。
