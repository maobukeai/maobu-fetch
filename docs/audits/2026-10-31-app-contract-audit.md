# 桌面应用前后端契约与键盘交互补充审查

检查日期：2026-10-31。检查范围：指定的三个 Tauri 命令、设置事件同步、通用 Select。**确认 5 项 P2 问题，均尚未修改生产代码。**

本报告依据仓库实际调用方、命令签名和锁定依赖实现；并将真实 `src/api.ts` 转译后在隔离 `invoke/listen` 替身上执行，捕获实际参数。没有启动真实 Tauri 实例，没有读取、恢复或改写用户数据库，没有访问 Tracker/下载链接。原生命令的最终行为按已核实的参数解析实现和本项目代码路径判断，未将这一检查当作原生端到端测试。

可重复检查：`node scripts/audit/app-contract.mjs`，源码见 [scripts/audit/app-contract.mjs](../../scripts/audit/app-contract.mjs)。

## 1. 命令参数规则的证据

[src-tauri/Cargo.lock:5502](../../src-tauri/Cargo.lock#L5502) 锁定 Tauri 2.11.5；[src-tauri/Cargo.lock:5602](../../src-tauri/Cargo.lock#L5602) 锁定 tauri-macros 2.6.3。本机 Cargo registry 中相应的 `tauri-macros-2.6.3/src/command/wrapper.rs` 已核实：

- 第 51 行默认 `argument_case: ArgumentCase::Camel`。
- 第 505—507 行对命令函数参数名调用 `to_lower_camel_case()`。
- 只有命令明确配置 `rename_all = "snake_case"` 才使用下划线参数名；下述三个命令均未配置。

同版本 `tauri-2.11.5/src/ipc/command.rs` 的 JSON 提取逻辑按最终参数 key 查找；必填字段缺失产生 `missing required key`，`Option<T>` 对应字段缺失则返回 `None`。因此错误不仅有“抛错”一种表现，也会静默忽略可选参数。

隔离运行 `api.ts` 捕获的实际载荷如下。这里的对象是传给 `invoke` 的真实 JS 参数结构；预期 key 来自上述命令生成器：

| 命令 | 实际 JS key | 原生命令预期 key | 结果 |
| --- | --- | --- | --- |
| `bt_inspect_torrent` | `torrent_path`、`torrent_base64` | `torrentPath`、`torrentBase64` | 两个 Option 参数被忽略；`source` 不受影响 |
| `task_refresh_url` | `new_url` | `newUrl` | 必填新地址参数缺失，函数不会正常执行 |
| `bt_update_trackers` | `subscribe_url` | `subscribeUrl` | Option 参数被忽略，后端改用已保存设置 |

## 2. 已确认的问题及准确影响范围

### APP-01 · P2 · “刷新下载链接”无法把新地址传给后端

定位：[src/api.ts:58](../../src/api.ts#L58)、[src-tauri/src/lib.rs:350](../../src-tauri/src/lib.rs#L350)、[src/components/modals/RefreshUrlDialog.tsx:81](../../src/components/modals/RefreshUrlDialog.tsx#L81)。

刷新对话框调用 `api.refreshTaskUrl()`；该函数向 `task_refresh_url` 传 `new_url`，原生命令读取必填 `newUrl`。隔离运行已捕获下划线 key，命令生成器确认不会读取它。

触发步骤：在已有任务上打开“刷新下载链接”，填入新 HTTP/HTTPS 地址并保存或保存后继续。实际在 IPC 参数提取阶段失败，新地址没有进入 `manager.refresh_url()`；对话框显示异常。两种提交方式都会经过同一调用，不应把它描述为仅“恢复下载偶尔失败”。

建议：前端改传正确 camelCase key，保留既有命令名。回归需覆盖“仅更新”和“更新并继续”，并在原生 IPC 边界检查确实接收到新地址；单独测试 manager 不足以发现此问题。

### APP-02 · P2 · 拖入种子的 Base64 内容预解析失效；磁力和有效本地路径有兜底

定位：[src/api.ts:17](../../src/api.ts#L17)、[src-tauri/src/lib.rs:155](../../src-tauri/src/lib.rs#L155)、[src/App.tsx:966](../../src/App.tsx#L966)、[src/components/modals/NewTaskDialog.tsx:322](../../src/components/modals/NewTaskDialog.tsx#L322)。

拖入文件时 App 保存 `{name, base64}`，新建对话框随后调用预解析；真实 Base64 被编码为 `torrent_base64`，原生命令预期 `torrentBase64`，因此忽略内容。对话框的 `source` 往往只是文件名，后端会按该名称尝试本地路径，并返回“无效的种子文件路径或磁力链接”（除非工作目录碰巧有同名有效文件）。种子内容预览与新建前文件选择因而失败。

**影响边界：**从系统文件选择器取得真实绝对路径时，调用同时携带正确命名的 `source`，后端仍可通过 `source` 解析；磁力链接也走 `source`，不受这两个错误 key 直接影响。真正添加任务用的是 `api.addBt()` 中的嵌套 `request.source_data_base64`，属于另一套 serde 数据结构，本次不一致不能推导为“所有 BT 添加/磁力下载都不能用”。

建议：修正这两个 invoke 参数名，增加“只有 Base64”“只有 torrentPath”“source 路径”“source 磁力”四种边界验证。保持嵌套请求结构与已有兼容字段稳定。

### APP-03 · P2 · Tracker 刚切换的订阅源被静默忽略

定位：[src/api.ts:61](../../src/api.ts#L61)、[src-tauri/src/lib.rs:360](../../src-tauri/src/lib.rs#L360)、[src/components/BtSettingsSection.tsx:88](../../src/components/BtSettingsSection.tsx#L88)、[src-tauri/src/manager.rs:1257](../../src-tauri/src/manager.rs#L1257)。

设置页在草稿中选择或输入新源后，“更新 Tracker”把 `currentUrl` 传给 `api.updateBtTrackers()`；真实调用载荷的 key 为 `subscribe_url`，原生命令忽略它，传给 manager 的是 `None`。manager 随即使用已保存的 `settings.bt_tracker_subscribe_url`。

触发步骤：打开设置，选一个与已保存设置不同的源，不先保存设置，直接点击更新。预期访问界面当前显示的新源，实际仍访问原来保存的源；原源不可用时还会错误表现为新源也不可用。若当前显示的源恰好就是已保存源，后端 fallback 能正常工作，不应报告成“Tracker 更新全部失效”。

建议：修正参数名；用两个不同的本地订阅端点验收实际请求地址。设置组件成功后会主动调用 `api.settings()` 回读 Tracker 列表，这是这一局部状态的补偿，见 APP-04 的范围说明。

### APP-04 · P2 · manager 发布的设置事件名称与 App 监听名称不同

定位：[src/api.ts:284](../../src/api.ts#L284)、[src/App.tsx:590](../../src/App.tsx#L590)、[src-tauri/src/manager.rs:218](../../src-tauri/src/manager.rs#L218)、[src-tauri/src/manager.rs:5093](../../src-tauri/src/manager.rs#L5093)。

实际 `api.subscribeSettings()` 监听 `settings-changed`；`save_settings()` 与 `backup_restore()` 发布 `settings-updated`。全仓搜索未发现 App 对后者的消费。因此只经过 manager 广播的变化不能更新 App 的内存 settings，直到明确刷新或重启。

受影响的确定入口与限制：

- **恢复含设置的备份：**后端已经替换内存与数据库设置，但当前 App/设置页仍可能保留旧设置，导致主题、完成提醒偏好等界面状态与后端不一致；继续保存旧草稿还可能覆盖刚恢复的设置。后半项是可达代码路径风险，本轮没有实际改写数据库复现覆盖。
- **后台自动设置变化：**例如计量网络恢复标志在 [src-tauri/src/manager.rs:633](../../src-tauri/src/manager.rs#L633) 和 [src-tauri/src/manager.rs:1422](../../src-tauri/src/manager.rs#L1422) 通过 `save_settings()` 写入。App 收不到同步；目前该内部标志没有直接 UI 展示，不能泛化成所有网络提示都错误。
- **普通设置页保存有直接补偿：**[src/components/settings/SettingsPage.tsx:534](../../src/components/settings/SettingsPage.tsx#L534) 成功后显式 `onChange(draft)`；主 App 能直接收到当前用户保存的值，因此不能报告成“点保存后设置都不更新”。
- **托盘切换有事件补偿：**[src-tauri/src/lib.rs:4317](../../src-tauri/src/lib.rs#L4317)、[src-tauri/src/lib.rs:4330](../../src-tauri/src/lib.rs#L4330)、[src-tauri/src/lib.rs:4346](../../src-tauri/src/lib.rs#L4346) 额外发布 `settings-changed`。这些已有入口不受名称不匹配直接影响。
- **备份恢复 custom event 只补偿快捷视图：**[src/components/BackupRestoreModal.tsx:112](../../src/components/BackupRestoreModal.tsx#L112) 发出 `maobu:backup-restored`，但 [src/App.tsx:514](../../src/App.tsx#L514) 的 `reload` 只读 `savedViewList()`，没有回读 settings。快捷视图的已有补偿应保留，不能据此认定恢复设置也已同步。
- **Tracker 局部列表有主动回读：**更新成功后组件调用 `api.settings()`，并将最新 `bt_extra_trackers` 写回草稿；这个列表不是完全依赖事件。它不修复 APP-03 的请求目标错误。

建议：统一设置事件合同，并考虑已有消费方的兼容期；备份恢复成功时回读权威设置，明确处理旧草稿。至少验证原生事件到 App 的实际传递，以及普通保存、托盘、后台修改、备份恢复这四种不同入口，避免修一个入口后错误推断全部正常。

### UI-01 · P2 · 通用 Select 无法仅用键盘选择其他选项，缺少选项语义

定位：[src/components/Select.tsx:99](../../src/components/Select.tsx#L99)、[src/components/Select.tsx:121](../../src/components/Select.tsx#L121)、[src/components/Select.tsx:163](../../src/components/Select.tsx#L163)。

组件只定制了 Escape 关闭；展开后每个选项是不可 Tab 聚焦的 `div`，只处理 click，没有 ArrowUp/ArrowDown/Home/End 或 Enter 选中逻辑，也没有 listbox/option、`aria-expanded`、`aria-selected` 等状态。

准确表现：原生 button 触发器仍可通过 Tab 聚焦、Enter/空格打开，不能说“整个控件完全无法键盘打开”；但展开后无法通过方向键移到其他选项并确认，Tab 也不会进入选项。受影响的是复用该组件的语言/主题/任务选项、Tracker 预设等下拉，鼠标选择仍有效。

建议：优先使用原生 select 或实现完整的组合框/listbox 键盘与焦点模型，确保禁用项跳过、展开/选中状态可被辅助技术识别、Escape 关闭并恢复焦点。此项为源代码确定的交互缺口，本轮未用屏幕阅读器或全键盘系统会话做完整验收。

## 3. 修复与验证要求

上述问题均无需更换 Tauri 命令名、清库或改变下载文件。三个错误是顶层 invoke 参数名，而非所有 Rust 字段都要改为 camelCase；`request` 等嵌套 serde 结构应遵循其现有约定。建议为顶层参数增加原生边界测试，并让前端/原生合同能共同校验；现有只检查 TS 类型或直接调用 Rust manager 的测试无法覆盖这类错配。

本报告仅覆盖指定入口，没有宣称已自动审计所有 Tauri 命令。5 项均为待修复问题；新增探针的成功运行表示观察可重复，不表示产品缺陷已经修复。
