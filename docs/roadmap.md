# 猫步下载器 修复路线图（动态看板）

> 由主控维护，随修复进度实时更新。最近更新：2026-08-17

## 🚀 2026-08-17 功能升级轮（本轮已全部落地并通过门禁）

| 编号 | 任务 | 状态 | 备注 |
| :--- | :--- | :--- | :--- |
| U-01 | 批量序号下载：`[001-100]`/`[start-end:step]` 模式展开接入新建对话框（url-sequence.ts 接线 + 挂载 check 门禁） | ✅ 完成 | 顺带修复磁力/本地种子行被 http 正则过滤导致徽标与 addBt 路径不可达的缺陷；测试 15/15 |
| U-02 | 分时段限速：每日时间窗口（可跨午夜）独立限速，`GetLocalTime` 零新依赖，30s 轮询同步 HTTP 内核与 aria2 | ✅ 完成 | bandwidth 窗口判定 7 项单测 + models 旧 JSON 兼容测试；设置页 time 输入 |
| U-03 | 快捷视图 SQLite 持久化：saved_views 表 + 4 命令 + localStorage 一次性迁移 + 纳入 AES 完整备份 | ✅ 完成 | store 往返/校验 3 项测试；恢复后经 maobu:backup-restored 事件刷新 UI |
| U-04 | 详情栏速度历史曲线：每秒采样真实 speed，纯 SVG 5 分钟窗口 | ✅ 完成 | 数据来自真实状态（§3）；i18n zh/en |
| U-05 | BT Tracker 列表：设置页多行输入 → aria2 `--bt-tracker`（启动参数 + 全局同步 + 每任务携带） | ✅ 完成 | bt:: 测试 37/37（含注释行过滤、参数构建） |
| U-06 | BT 边下边看：`bt-prioritize-piece=head=16M,tail=16M`（语法对照 aria2 1.37 手册核实） | ✅ 完成 | 新建对话框勾选项，BtTaskMeta 持久化（serde 默认兼容旧库） |
| U-07 | 收尾：updater 过时 TODO 注释清理（地址本就正确）、README 版本/yt-dlp 版本/描述更新、.gitignore 补 extension.zip | ✅ 完成 | F-11 已核实接线（lib.rs build_extension_compatibility_result）、F-12 已解决 |

**门禁结果（2026-08-17）**：`pnpm run check`（i18n 19 + 序号 15 + 拖放 6 + 扩展 131 全过）、`pnpm run build` ✓、`cargo test --lib` 1135 全过、`pnpm run extension:build` ✓。全目标 `cargo test` 因运行中的调试实例锁定 maobu-fetch.exe 未能重链 bin（入口为薄启动器，风险低），待实例退出后补跑。

**UI 渲染验证（2026-08-17，浏览器模式 DOM 级）**：新建任务对话框已验证——新 aria-label/placeholder 生效；输入 magnet: 后"磁力任务"徽标出现、"已检测到 1 个链接"计数正确、"创建后暂停/边下边看"双复选框带 tooltip 渲染、提交按钮解锁（确认 magnet 过滤修复有效）；恢复预览弹窗已补充"应用快捷视图 N 条"。**未验证**：设置页分时段限速/Tracker 输入的实际渲染（浏览器环境点击管线不稳定未能导航）、深浅色与 125% 缩放像素级检查、BT 真机行为——需在桌面端 dev 实例人工过一遍。附带修复：api.ts 浏览器回退 mock 的 yt-dlp 版本号 2026.06.09 → 2026.07.04。

**细微复查轮（2026-08-17 第二轮）**：修复 url-sequence.ts 超大区间先物化数组再查上限的 UI 冻结/OOM 隐患（[1-999999999] 现立即报错，回归测试 +2）；修复剪贴板监视只匹配 http(s) 导致磁力接管从不触发的缺口（bt_intercept_magnet 语义包含剪贴板，已接线并尊重开关）；修复 .bt-settings-row-wide CSS 定义先于基类被覆盖的布局缺陷；速度采样无选中任务时停止空转。候选：CLI `add` 目前仅支持 HTTP/HTTPS（有明确错误提示），BT 支持需完整 AppHandle 上下文，列为后续任务。

**遗留债务（已量化，待专项）**：App.tsx 存量硬编码中文约 460 行用户可见文案（JSX 文本 210 / 属性 169 / 错误与 toast 80），本轮新增文案已全部国际化；`manager.rs`/`App.tsx` 巨型文件拆分待排期；BT 真机（aria2 实下载/做种停止/会话恢复）仍待验证。

## 📌 P0 基础核心功能（当前修复主线）

| 编号 | 任务 | 状态 | 负责 |
| :--- | :--- | :--- | :--- |
| F-01 | 修复 v0.6.8 扩展接管回归：第二次评估把扩展自身暂停的下载误判为 restored-history，接管 100% 失效 | 🔧 进行中 | agy-frontend-engineer |
| F-02 | 通知节流时间戳持久化到 chrome.storage.session；补齐未节流通知（取消失败） | 🔧 进行中 | agy-frontend-engineer |
| F-03 | 拦截链路前置配对预检；未配对时单次节流引导通知，不再每次下载报错 | 🔧 进行中 | agy-frontend-engineer |
| F-04 | App.tsx 补 `notification-focus-task` 事件监听，收尾 Windows 原生 Toast 工作 | 🔧 进行中 | agy-frontend-engineer |
| F-05 | bridge.rs `/v1/pair` 增加速率限制（当前可无限重试爆破配对码） | 🔧 进行中 | agy-backend-engineer |
| F-06 | CDN 连接数上限的"用户显式设置"判定由启发式改为显式标志字段 | 🔧 进行中 | agy-backend-engineer |

## 🐛 P1 缺陷与自愈队列

| 编号 | 任务 | 状态 | 负责 |
| :--- | :--- | :--- | :--- |
| F-07 | 扩展测试 mock 的 downloads.search 不反映 pause 后状态（掩盖 F-01 回归），修复并补回归测试 | 🔧 进行中 | agy-frontend-engineer |
| F-08 | popup 诊断 reasonMap 缺 restored-history / unpaired 等中文文案 | 🔧 进行中 | agy-frontend-engineer |

## 💎 P2 工程打磨

| 编号 | 任务 | 状态 | 负责 |
| :--- | :--- | :--- | :--- |
| F-09 | 统一验证门禁：`pnpm run check` / `pnpm run build` / `cargo test` / `pnpm run extension:build` + Diff 审查 | ⏳ 待派发 | agy-qa-engineer |

## 🚀 P3 自主演进候选（本轮不做，待负责人确认）

- F-10 桌面端"允许扩展接管下载 / 最小文件大小"设置从未下发到扩展（需 `/v1/health` 增量字段 + 扩展读取，涉及 Rust/扩展/协议测试三方同步）
- F-11 updater.rs `check_extension_compatibility` 未接线
- F-12 仓库根存在未跟踪 `extension/extension.zip`（按规范勿提交）
- F-13 观察 F-03 落地后 401 清除令牌的实际体验，必要时改为 health 二次确认后清除

## 🧲 BT/磁力下载（2026-08-16 负责人批准纳入，执行中）

架构决策：aria2 未修改官方构建作为按需安装组件（复用 media_tools 机制），RPC 仅监听 127.0.0.1 随机端口 + 随机 secret；做种默认关闭；磁力元数据获取前不得伪造文件名/大小。约束详见 AGENTS.md §3“BT/磁力内核”与 §6。

| 编号 | 任务 | 状态 | 负责 |
| :--- | :--- | :--- | :--- |
| BT-01 | media_tools 扩展：aria2 1.37.0 固定版本、SHA-256 清单、按需安装/卸载（仅提取 aria2c.exe + COPYING + 源码链接） | ✅ 完成（含单测） | backend |
| BT-02 | aria2 进程管理：随机端口 + `--rpc-secret` 启动、健康检查、优雅退出与会话保存（程序退出钩子接入） | ✅ 完成（含单测） | backend |
| BT-03 | aria2 RPC 客户端（JSON-RPC over 127.0.0.1，no_proxy 强制，令牌 Debug/Display 脱敏） | ✅ 完成（含单测） | backend |
| BT-04 | 任务模型：`task_kind` 字段、磁力 URI 解析（hex/base32/v2 拒绝）、SQLite 增量迁移（旧数据默认 http，含迁移+重启往返测试） | ✅ 完成 | backend |
| BT-05 | manager 生命周期：添加磁力/种子、元数据阶段、文件勾选、暂停/恢复/删除（双选项）、完成即停做种（seed-time=0 默认） | ✅ 完成 | backend |
| BT-06 | 全局限速映射到 aria2（changeGlobalOption 下载/上传 + 每任务 seed 策略下发） | ✅ 完成 | backend |
| BT-07 | 前端：新建对话框磁力/种子识别徽标 + .torrent 选择按钮、任务表 BT 徽标与"待获取"、详情栏 BT 页签（真实 peers/seeds/上传速度 + 文件勾选）、设置页 BT 分区（含隐私说明）、i18n zh/en | ✅ 完成 | frontend |
| BT-08 | 扩展：`magnet:` 链接点击/键盘接管（send-magnet 契约、桌面设置经 /v1/health 同步、离线/未配对/关闭接管安全回退、划词提取含磁力） | ✅ 完成（links.js 补全角标点修复与测试） | frontend |
| BT-09 | 测试：磁力解析 10 例、RPC 构建/解析/脱敏、状态映射（METADATA 阶段）、迁移+重启往返、扩展 links 5 例与全角标点；cargo 1117 全过 / extension 69 全过 | ✅ 完成（真机 aria2 下载行为待验证） | qa |
