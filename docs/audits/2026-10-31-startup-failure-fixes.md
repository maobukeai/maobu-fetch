# 损坏设置启动失败：错误传播与数据保留

日期：2026-10-31（Asia/Shanghai）。范围仅限已经确认的启动错误路径，不改下载、桥协议或媒体源选择。

## 已确认的链路

设置读取现已对损坏 JSON/错误字段类型返回中文 Err，保留原记录；`DownloadManager::new` 首先调用 `store.get_settings().await?`。原 GUI setup 的 Store/Manager 初始化错误继续传到 `Builder.build`，但末尾的 `expect("error while building Maobu Fetch")` 将可恢复错误转为 panic。Windows release 是无控制台子系统，用户可能看不到原始中文原因。

已先核对现有机制：日志在 Store 之前初始化，已有脱敏 writer；Tauri dialog 插件依赖可用 AppHandle，不能承担 Builder 创建失败时的兜底提示。

## 最小修复

- 新增 `startup_failure` 小模块，production setup 与测试共同调用相同 `initialize_manager`，正常传播 Store/Manager 错误，没有默认配置替换或自动修复。SQLite 打开与旧数据迁移在 `spawn_blocking` 中执行，后台任务 JoinError 与 Store 错误都明确返回中文 Err；之后再异步创建 DownloadManager。
- `Builder.build` 的实际 Result 进入 `finish_build` 的显式 match。成功才运行 App；失败先脱敏、移除危险控制字符并限制详情长度，输出可操作中文原因和实际数据目录，返回退出码 1，保留错误而不 panic。
- GUI 模式使用已有 windows-sys 的 `MessageBoxW`，无需已成功创建的 Tauri App；仅给现有依赖启用 WindowsAndMessaging feature，没有新增依赖。UTF-16 缓冲保持存活并仅尾部 NUL 终止，提示复制错误、检查目录权限/空间、备份后恢复，不建议删除数据库。
- 既有日志订阅者可记录已经脱敏的错误，stderr 同步输出；没有新增修复文件或数据库/设置写入。原启动的正常增量迁移行为未重写，也没有凭启动失败去删除原数据。
- `run` 返回明确退出码，main 原样退出；CLI 参数解析失败仍为 2，既有 CLI 执行语义/命令名保持。已有 GUI 的 CLI 转发使用控制台模式并附加父控制台，启动失败只写 stderr，不弹模态 GUI；正常 GUI/播放器/看图器启动失败才展示原生提示。

## 验证设计与当前执行状态

首轮 full11 的实际结果是 **1476 通过 / 1 失败 / 16 忽略**。失败暴露新初始化代码和其临时数据 fixture 在异步 runtime 中直接调用 `Store::open`，触发 Store 旧数据迁移的 `blocking_lock` panic。此失败保留，不记为通过。生产初始化和 fixture 均已改用 `spawn_blocking`，并新增有效配置真实初始化成功的对照场景，避免只修失败 fixture。

恢复时另发现 `startup_failure.rs` 与 `startup_cases.rs` 的内容全为 NUL；按现存调用与完整的 native/tests 子模块重建。写入及格式化后四个负责的 Rust 文件逐一检查，均无 NUL。没有恢复、清理或覆盖其他用户文件。

修复后专项编译 `cargo test --manifest-path src-tauri/Cargo.toml --lib startup_failure::tests --no-run` 成功（36.99 秒）。复制新测试 EXE 到唯一 TEMP 目录后，6 项 FakeSink 单元测试 **6/6 通过**（0.01 秒）；以 `MAOBU_FETCH_ISOLATED_TEST=1`、`MAOBU_TEST_NATIVE_ONLY=startup-failure` 运行实际原生聚合 selector，**1/1 通过**（0.10 秒），内部依次执行以下 4 个真实初始化场景。日志位于本次 TEMP `maobu-startup-repair-fe1bf5a58c694bff9e294c5c1de8d117` 的 `unit.log`、`native.log`；专项编译日志为 `%TEMP%/maobu-startup-repair-compile.log`。之后根任务 full12 实际通过：**1477 通过 / 0 失败 / 16 忽略**（98.48 秒），Range 集成测试 **9 通过 / 0 失败**（5.06 秒）；日志为 `%TEMP%/maobu-final-cargo-round12.log`。根任务对 279 份 Rust 源码快照核对，Changed 为 0。**包含本修复的 release round4 已实际构建成功（exit 0，4 分 44 秒），全新 TEMP 产物静态验收 13/13 通过；实际产物、SHA 与静态验证边界见[发布产物验收报告](./2026-10-31-release-inspection.md)。**

6 项 FakeSink 回归覆盖：成功 callback 只执行一次且返回 0；失败返回 1、没有运行 App、无 panic；CLI 不弹窗；Cookie/Authorization/代理密码/令牌在两个 sink 前完成脱敏；原生提示失败仍保持失败退出且次级原因脱敏；长中文/NUL/控制字符及 UTF-16 缓冲边界。

现有单 Wry 聚合新增 `startup-failure` 诊断 selector，默认完整聚合也会执行 4 个实际临时数据场景：损坏 JSON、错误字段类型、损坏 SQLite 字节、有效配置正常启动。前三者调用正式 `initialize_manager`，实际进入 Store/Manager 初始化，错误按 Tauri Setup Error 类型封装后交给同一 `finish_build`。只使用 FakeSink，验证未进入 App callback、无 panic、退出码 1、原 JSON 精确保留、原暂停任务文件名/状态/数量保留、坏数据库原字节保留，且失败发生在 BT session 初始化之前。第四个验证实际异步初始化成功、私有下载目录与关闭自启的设置保留、成功 callback 恰好一次并返回 0、没有错误提示；最后通过正式 shutdown 停止管理器，并再次确认原任务及空控制表。

测试复用已经存在的隔离 Wry AppHandle，不创建第二个事件循环，不注册文件关联或真实自启，不读取个人数据库/Cookie。它没有在宿主机运行真实 GUI 启动失败，也不冒称真实 MessageBox 的视觉交互已经通过。

## 产物与未验证边界

release round3 的 build/fresh 13 项与原始 SHA 保持真实历史结果，但没有包含本补强，不能当作最新生产源码产物。当前候选为包含本修复的 round4：实际构建成功，之后在全新 TEMP 目录静态验收 13/13 通过，发布专项已更新当前表并保留旧轮记录。这证明最新源码已进入本轮静态验收产物，不替代原生运行与安装生命周期验证。

用户尚未恢复桌面操作授权，因此未执行 NativeSink 的真实 MessageBox，不弹测试对话框、不启动 release EXE或安装器。该原生提示的实际显示/关闭与 release `/v1/health`、隔离安装升级/卸载仍按未验证记录，不能用 FakeSink 代替。
