# 猫步下载器 · Maobu Fetch

猫步下载器（Maobu Fetch）是面向 Windows 10/11 的开源效率型下载器。品牌中的 “Fetch” 同时表达下载获取与猫取回物品的动作；界面采用紧凑、克制的 Windows 11 风格，不包含广告、在线字体或云端服务。

## 已实现

- SQLite 任务与设置存储，并兼容迁移旧版数据
- 每任务可选 1、2、4、8、16 路 HTTP Range 分段连接，实时显示各分段进度
- 全局并发、优先级队列、全局限速下按高/普通/低 4:2:1 加权分配带宽、计划任务、批量暂停/继续与指数退避重试
- ETag/Last-Modified 变化检测、持久续传、原子合并与 SHA-256 校验
- 全局及单任务限速、重名策略、自定义请求头、Referer、Cookie 与 Authorization
- 可排序任务表、多选、快捷键、详情栏、真实速度与剩余时间、深浅色主题
- Chrome/Edge Manifest V3 扩展：右键下载、下载接管、临时绕过与页面媒体发现
- 本地 `/v1` 桥：一次性配对码、持久令牌、精确 Origin、HMAC 签名与速率限制
- 按需安装并校验 yt-dlp 2026.06.09 与 FFmpeg 8.1.2；支持媒体探测、格式选择、字幕和合并；拒绝 DRM

BT、磁力、账号同步、远程下载、DRM 绕过和 Firefox 不在当前范围内。

## 开发与构建

需要 Node.js 20+、pnpm 11+、Rust，以及安装“使用 C++ 的桌面开发”组件的 Visual Studio 2022 Build Tools。

参与开发前必须阅读并遵守 [`AGENTS.md`](AGENTS.md) 中的开发注意事项与强约束。

```powershell
pnpm install
pnpm desktop:dev
```

构建浏览器扩展后，在 Chrome/Edge 的扩展管理页加载 `extension/dist`：

```powershell
pnpm extension:build
```

桌面端启动后，在“设置 → 浏览器”查看一次性配对码，并在扩展弹窗输入。未连接或未配对时，浏览器下载不会被取消。

## 验证

```powershell
pnpm check
cargo test --manifest-path src-tauri\Cargo.toml
pnpm tauri build
```

## 许可

MIT。猫步下载器与 Neat Download Manager 无隶属关系，也未使用其专有源码或素材。
