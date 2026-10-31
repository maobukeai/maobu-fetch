# M14：兼容矩阵验证证据与旧数据修复

日期：2026-10-31。范围：内置兼容矩阵、SQLite 增量迁移、读取 API、设置页兼容区域。没有新增后台联网扫描，没有将本地 fixtures 当作站点实测。

## 原问题和额外缺陷

- 多个平台内置为 `verified`，但 `last_tested_at` 为空，也没有软件/工具版本、样本和验证层级。
- 原种子 SQL 对 `pan123`、`lanzou` 无条件更新，重复打开数据库会覆盖用户自定义说明和级别。
- 内置文案声称“全功能正常”“均可正常下载”“极速并发”等，未与可追溯样本证据绑定。

## 最终规则

只有声明为 verified，并同时提供以下完整证据的记录，读取 API 才返回 `Verified`：

1. `app_version` 与当前运行软件版本完全一致。
2. `last_tested_at` 为有效、非未来的日期，支持 `YYYY-MM-DD` 或 UTC 秒精度时间。
3. `tool_version` 和 `sample` 均有内容。
4. `scope=download` 且 `result=passed`，表示完整下载样本通过。

元数据解析、合并、登录/权限和播放检查分别记录为 `metadata / merge / login / playback`。它们即使成功，也不能外推为完整下载已验证。旧版本的成功样本显示历史记录，不给当前版本的已验证徽章。所有内置记录现在只提供静态入口说明，默认无证据，设置页明确显示“未验证”。

新增可选 `PlatformVerificationEvidence`，缺失字段安全默认；沿用已有 `last_tested_at`。数据库仅增量添加可空 `verification_json` 列，不删除、重建或清空用户表。迁移与原有 schema/seed 事务一起提交，失败整体回滚。

## 用户自定义和旧版本兼容

- 只在平台、旧内置原文、空日期、空问题列表、无证据、旧 verified/experimental 状态均符合时，才把旧内置文案精准迁移为中性说明。
- 后续默认记录使用 `INSERT OR IGNORE`，不再按平台名无条件覆盖。
- 用户自定义 notes、known_issues、Unsupported 级别和原始 Verified 声明均保留在数据库。自定义 Verified 缺证据时，API/界面展示为未验证，不改写其原始声明。
- JSON 损坏或字段缺失不会使列表崩溃，也不会形成 Verified。
- 没有复制媒体审查中的部分 metadata 成功结果作为“当前版本完整下载通过”的证据，没有填入虚构日期、版本、样本或校验值。

## 代码范围

- `src-tauri/src/store/platform_compatibility/`：独立 builtins、证据规则和迁移/重启测试。
- `src-tauri/src/store/platform_compatibility.rs`：增量迁移入口与统一 list/get 读取。
- `src-tauri/src/models.rs`、`src/types.ts`：可选证据字段及安全默认，由主任务集成。
- `PlatformCompatibilitySection.tsx`：从原设置大文件抽出的紧凑表格，支持查看日期、软件/工具版本、样本和层级。
- `platform-compatibility.ts`：前端额外守门，旧后端的无证据 verified 也不会直接变为“已验证”。

## 验证结果

| 验证 | 实际结果 |
| --- | --- |
| TypeScript 证据展示回归 | 通过：无证据、完整样本、软件版本不符、缺运行版本、坏日期、缺样本、metadata-only、失败结果 |
| `npx tsc --noEmit` | 退出 0 |
| 实际 Edge 组件视觉/键盘 | 9 项通过：1280×720、1440×900，100%/125%，浅/深色共 8 组合，另有空列表；长样本不越过视口，键盘 Enter 展开证据，零 pageerror |
| 人工图片检查 | 已看浅/深色 125% 截图；修正深色 summary 焦点框，复验为清晰 accent 框；正文不以低对比彩色字承担状态含义 |
| SQLite 旧 5 列库真实升级、3 次重启、自定义保护、证据失效及损坏、触发器故障回滚 | `cargo test --manifest-path src-tauri/Cargo.toml --lib platform_compatibility -- --nocapture`：**11 passed / 0 failed / 0 ignored**，0.30 秒；包括旧测试修正与新增真实迁移场景 |

视觉命令：`node scripts/audit/compatibility-ui.mjs`。本轮最后截图和结果位于系统临时目录 `maobu-compatibility-ui-MVrSO5`，Rust 日志为 `%TEMP%/maobu-compatibility-tests.log`，没有提交到仓库。

## 验证边界

这些测试验证“兼容结论是否有证据、如何保存和展示”，不验证各站点当前是否能下载。UI 使用隔离的合成证据，不写入桌面数据库、不触发 native 命令、无用户账号或 Cookie。完整设置窗口、其余设置区域、打包升级与最终全部测试仍由主任务验收；不得把 9 项组件验收说成整个软件的全部视觉检查。
