# 浏览器扩展整改与回归记录

日期：2026-10-31（Asia/Shanghai）。对应审查：`2026-10-31-extension-audit.md` 的 EXT-01～07，以及总报告 REL-03。

本文件记录代码整改和已执行证据，不把局部测试当作整个产品已通过发布验收。未修改用户日常浏览器配置、真实 Cookie、任务或已安装扩展目录；浏览器测试全部使用临时配置和合成凭证。

## 1. 逐项整改

| 编号 | 修改 | 验证 |
| --- | --- | --- |
| EXT-01 | 网页 LanDisk 消息只提出请求；后台要求当前顶层文档、私网/Tailscale 或用户明确允许的域、严格同源 `/api/download/batch`、合法文件表单、接管/绕过规则、在线且已配对。真正创建任务前打开扩展自身 origin 的安全确认窗口，绑定原始请求和确认窗口 Tab；页面不能批准、合成 `.click()` 不生效。关闭/超时/导航会回退网页下载。确认后再次校验来源 documentId 和设置；不接收任意请求头。 | 策略负向测试、一次性确认/重放/错误窗口/源导航测试；实际 Chrome/Edge 网页→扩展确认窗口→拒绝→可信点击批准→POST 载荷流程。 |
| EXT-02 | 新建 `cookie-context.js`，按最终目标 URL 读取 Cookie，不合并来源页/Referrer Cookie，不把来源 query token/pin 转为目标头。保留同名不同 path Cookie 的路径顺序；Cookie 导出保留 domain/path/hostOnly/secure，不把分区 Cookie 降格成未分区凭据。 | 跨域、重定向最终目标、同名 path、精确 partition、一次性认证下载和导出范围测试；双浏览器实际 Cookie API 跨主机测试。 |
| EXT-03 | 使用 `cookies.getAllCookieStores().tabIds` 解析真实 store，使用 `getPartitionKey` 确认 frame 分区，不依赖 Tab.cookieStoreId；无法确认时不猜普通身份。无痕标签不自动同步进持久凭证库，主动的一次性认证下载/手动导出仍可用。 | 正常/无痕 store 单测、自动同步负向测试；实际 Chrome/Edge 标准无痕窗口分别放置合成 Cookie，读取 PRIVATE、不读取 NORMAL。 |
| EXT-04 | 保留真正取消动作；主 agent 在 Rust 桥增加已有 cancel 的兼容支持。扩展对旧桌面端明确返回“不支持的操作”时降级为暂停，并显示实际暂停结果，不假称撤销成功。其他取消错误不会被掩盖。 | 新/旧动作响应测试；Rust协议测试与真实Chrome/Edge→Rust隔离联调已通过，实际取消后状态cancelled且无完成文件。 |
| EXT-05 | 不再把用户的 1 MB 设置重写为 0；桌面端阈值同样保留。ZIP/安装包不再静默豁免，未知体积在有阈值时交回浏览器；用户明确记住“始终接管”的主动选择维持现有语义。 | 旧错误测试改为正确约束，新增未知大小场景；后台探针确认 1 MB 未被修改、10 字节 ZIP 不越过 100 MB 阈值。 |
| EXT-06 | 磁力的发送错误（含 401/5xx）均交回浏览器处理；同时尊重总开关、磁力开关、临时绕过、桌面端设置。 | 既有离线/未配对/开关测试；后台真实源文件替身复现 401，结果 fallback=true。 |
| EXT-07 | 主文档、SPA history/hash、重新加载和标签关闭均清空嗅探；维护当前 frame/document 树，拒绝旧文档晚到请求，同时保留合法 iframe 媒体；停止监听时解除全部事件。缓存不包含历史 URL，仍仅内存。 | 主文档/SPA/晚到请求测试、iframe 子树替换测试、7 类监听清理测试；审查探针确认导航后无旧媒体。 |
| REL-03 | 构建只写脚本所在扩展目录下规范化的 dist，不再读取/覆盖 APPDATA 安装位置；从任意 cwd 执行也不会删除 cwd 下的 dist。错误正常返回。 | 以临时 APPDATA 放置 manifest/旧脚本哨兵，从外部 cwd 执行真实构建，哨兵保持不变。 |

## 2. 代码与兼容

- 认证：`extension/src/cookie-context.js`、`auth-download.js`、`interceptor.js`、`background.js`、`popup.js`。
- LanDisk：`landisk-security.js`、`landisk-approval.js`、确认页 HTML/CSS/JS、隔离世界 `landisk-bridge.js`、保留网页侧打包钩子的 `landisk-adapter.js`。去除了重复的任意多任务转发入口；现有网页打包下载和猫步按钮保留，创建前需明确确认。
- 任务取消：`task-action.js` 和内容徽章反馈。
- 嗅探：`sniffer-documents.js` 与 `sniffer.js`。
- 构建：`extension/build.mjs`；`extension/package.json` 补新模块语法检查。

`/v1` API 版本、签名算法、X-Luma-* 头、端口和命令名保持不变。cancel 是对已有 action 值的兼容扩展，老客户端有明确暂停降级。后续直播交互闭环仅在已有 `media` 对象追加可选 `is_live`：缺失默认 false，旧桌面端忽略未知字段；该值只作初始展示，服务端停止录制仍要求本代真实探测。扩展 true/缺失字段回归已覆盖。版本号未更改，尚未发布。

manifest 新增 `webNavigation`，仅用于识别当前主文档/iframe/SPA 导航和确认来源未被替换；只存当前 documentId/frameId，未保存浏览历史。扩展更新后需重载并按浏览器要求处理权限变化。浏览器测试未改用户的实际扩展授权。

## 3. 实际执行

- `npm --prefix extension run check`：**185 通过 / 0 失败**，包含语法检查、原有协议/配对/离线/绕过测试和新增回归。保留并纠正固化旧缺陷的阈值/Cookie 预期，没有删除失败用例绕过错误。LanDisk 旧测试原先复制产品逻辑，现改为实际读取并执行原始构造函数。
- `node scripts/audit-extension.mjs`：双浏览器通过。脚本现在对预期安全结果作明确断言，缺陷再次出现会返回非零，不再仅打印观察值。
- Edge `156.0.4314.50`、Chrome `154.0.8037.92`：真实扩展 GET/POST Origin 和 HMAC 正常；源主机 Cookie 没有发给另一主机；标准无痕 Cookie 来源正确。
- LanDisk：在本地合成网页执行真实 MAIN/ISOLATED 脚本与后台消息；确认前 0 任务，脚本 `.click()` 无法批准，真实浏览器点击批准后正确产生单个同源 POST 请求，并带正确合成 Cookie。原生 17433 的 fetch 在临时 Service Worker 内由替身承接，**没有向正在运行的桌面端发送任务**。
- 确认窗口：双浏览器各浅/深色、100%/125% 共 **8 张截图**；440×300 压力视口、长打包名和 5 条长路径，操作按钮均在视口内，已目检。不是更改 Windows 系统 DPI。
- `git diff --check`：无空白错误；本机 LF/CRLF 转换提示不属于失败。

浏览器最终重跑证据为 `%TEMP%/maobu-extension-audit-WvMEo7/evidence.json`；已目检批次为 `%TEMP%/maobu-extension-audit-XqSTDL/evidence.json`，截图在同目录。检查日志：`%TEMP%/maobu-extension-fixes-check.log`。随后能力/码率/原文件/未知音轨、真实 HTTP 503 保留配对令牌、可选直播标记回归补齐，最后重新执行最新为 **193/193**，见 `%TEMP%/maobu-extension-media-check.log` 与媒体选择修复记录。

使用标准 `chrome.windows.create({incognito:true})` 验证无痕 store；CDP 自建的额外 OTR context 不等同于用户标准无痕窗口，不能把其不可见 store 当作功能回归。

## 4. 尚需产品整体验收

本轮没有使用真实账号，也没有执行商店分发/企业策略/旧 Chrome 与 Edge 全版本矩阵。真实扩展→Rust随机本地端口的配对/暂停/恢复/取消/完成SHA/撤销与桥离线后的实际浏览器下载回退已经通过，见 `2026-10-31-bridge-browser-validation.md`。正式release端口、浏览器普通下载自动接管、浏览器崩溃恢复、登录站点完整媒体下载仍需对应验收。确认页和 Cookie 修复不能替代 HTTP/BT 内核完整性修复，也不能证明 14 平台全部可下载。
