# APP 补充：Windows 文件关联错误与测试隔离

日期：2026-10-31。范围：`file_assoc.rs`、注入注册表根的 helper、独立 HKCU 测试键与系统集成隔离入口。

## 确认的问题

1. 原启用、撤销和基础登记包含大量 `if let Ok` / `let _`；创建项、写命令、图标、Capabilities 或删除值失败后仍返回成功。
2. 未知扩展名会回退为视频 ProgID，而非在写入前拒绝。
3. 撤销没有清理本应用的 `OpenWithList` 项，可能留下候选应用注册。
4. 如果测试或临时 EXE 执行真实 App setup，原 ensure 会把本应用命令/图标路径登记为该临时 EXE。它是确认的代码路径风险；本会话未执行真实 setup，不能推定文件关联已经被改写。先前实际发生的自启 Run 测试副作用及恢复另见 `2026-10-31-test-isolation.md`。

## 修改

- 注册、查询和删除使用统一、可注入的用户根键；只有 NotFound 可按不存在处理，权限等失败沿 Result 返回可操作中文原因。
- 全批扩展名在任何登记前校验，支持原有 9 种视频和 9 种图片，统一大小写、去重；未知格式整批失败。
- 原有 `MaobuFetch.Video`、`MaobuFetch.Image`、`Applications\maobu-fetch.exe`、`MaobuFetch`、`app.lumaget.desktop` 等标识全部保留。
- 撤销删除本应用候选映射和 `OpenWithList` 子项；扩展名默认值、Explorer UserChoice/Hash 不修改。ProgID 和基础能力保持可用于现有默认应用配置；注销候选应用不冒充修改系统默认应用。
- `open_default_apps_settings` 不再吞掉系统面板启动失败。
- 顶层调用 `system_integration::allowed()`。cfg(test) 或显式隔离环境中，ensure 无副作用返回 Ok，显式 set/open 返回中文隔离提示；查询也不读取真实 Classes。

## 实际行为验证

运行：`cargo test --manifest-path src-tauri/Cargo.toml --lib file_assoc -- --nocapture`。

结果：**8 passed / 0 failed / 0 ignored**。真实 registry helper 直接操作每个测试独有的 `HKCU\Software\MaobuFetchTests\file-assoc-UUID`，不是把公开入口的隔离跳过当作功能通过。

| 场景 | 证据 |
| --- | --- |
| 视频/图片启用、重复启用、撤销、重复撤销 | 真实私有键内核验 ProgID、带空格/中文路径命令、候选映射与最终查询状态 |
| 未知扩展名、路径式输入、空输入项 | 返回失败，测试根尚无子项，没有部分登记 |
| 基础 Applications/Capabilities | 原 RegisteredApplications 值、18 种 SupportedTypes 与 open 命令正确 |
| 权限拒绝 | 私有键临时设置 Everyone deny DACL，先确认 Windows 返回 AccessDenied，再确认 ensure/set 不报成功 |
| 删除权限拒绝 | 拒绝目标叶键 KEY_SET_VALUE，撤销明确失败，原关联仍可查询；恢复 DACL 后重试成功 |
| 默认应用保护 | 私有模拟默认 ProgID、UserChoice 和 Hash 在启停后原值不变 |
| 隔离与清理 | 真实应用 Run 值只读快照前后相同且不输出内容；DACL 明确恢复后验证写入成功，UUID 根删除并核验不存在 |
| 自启与通用隔离入口 | 同一已编译测试 EXE direct filters：autostart **3 passed**、system_integration **1 passed**；自启真实行为也仅在专属 UUID 键执行 |

日志：`%TEMP%/maobu-file-assoc-tests.log`。Windows DACL fixture 仅用于测试，生产路径使用 winreg 的普通 Result 处理。

## 边界

- 测试没有操作真实 `Software\Classes`、`RegisteredApplications`，也没有更改真实默认应用。
- 本应用已有真实注册命令是否指向用户期望的正式安装/便携路径，不能由私有根测试证明；主任务在正式烟测前只读核验，不盲目覆盖用户自定义。
- 系统“默认应用”面板需要由用户主动打开并选择；隔离测试不会启动该面板，也不宣称完成了系统 GUI 自动默认选择。
- 以上两组最初重编译曾被媒体测试新增类型错误阻塞；错误修复后使用新测试 EXE 实际执行通过，日志为 `%TEMP%/maobu-autostart-direct-tests.log` 与 `%TEMP%/maobu-system-direct-tests.log`。编译受阻轮次没有算作通过。
