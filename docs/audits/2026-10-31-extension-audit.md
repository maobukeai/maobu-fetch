# 浏览器扩展与本地桥深度检查报告

检查日期：2026-10-31（Asia/Shanghai）。项目版本：0.10.11。

本报告记录 **7 项产品问题（3 项 P1、4 项 P2）和 1 项测试体系缺口**。本轮只审查扩展与协议，未修改生产扩展代码；新增的是隔离审计脚本。优先级含义：P1 应在继续分发前优先处理；P2 应纳入下一轮功能修复。

## 1. 实际执行与证据边界

| 项目 | 实际结果 | 能证明什么 |
| --- | --- | --- |
| `npm --prefix extension run check` | 169 个模拟 Chrome API/纯函数测试通过 | 现有测试可运行；不等于全部实际功能正确 |
| Chrome 154.0.8037.92 | 全新临时配置，成功通过 CDP 加载仓库实际 MV3 扩展 | manifest、Service Worker、扩展页面和真实扩展 API 可使用 |
| Edge 156.0.4314.50 | 全新临时配置，成功加载相同扩展 | Edge 同样可以运行这份扩展 |
| 实际扩展 GET/POST | 向临时本地回显服务请求，Origin 存在；独立计算的 HMAC 均匹配 | 浏览器真实发送结果与 JS 签名协议一致 |
| Cookie 作用域 | 两个真实浏览器均复现源页面 Cookie 被拼进另一主机目标的下载头 | 并非只存在于模拟 Chrome API |
| 正常/无痕隔离 | 两浏览器的无痕 Tab 均没有 `cookieStoreId`；帮助函数读出正常上下文的合成 Cookie | 当前无痕实现无法可靠选中正确 cookie store |
| LanDisk 页面桥、后台入口、体积阈值、磁力 401、嗅探导航缓存 | 已用真实源文件 + 隔离 API/网络替身复现 | 可重复观察相应分支；未向真实桌面端创建任务 |

可重复命令：

```powershell
node scripts/audit-extension.mjs
# 只运行本地源文件与隔离后台探针，不启动浏览器：
node scripts/audit-extension.mjs --skip-browsers
```

入口为 [scripts/audit-extension.mjs](../../scripts/audit-extension.mjs)，具体探针分为 [本地边界](../../scripts/audit/extension-local-probes.mjs)、[后台事件](../../scripts/audit/extension-background-probes.mjs)、[真实浏览器 API](../../scripts/audit/extension-browser-probes.mjs)。脚本输出中的 `true` 可能表示“缺陷已复现”，并非“测试通过”。浏览器不存在或探针无法执行时会标记 `unverified`，不会当成通过。

本轮最终证据文件位于系统 TEMP 下的 `maobu-extension-audit-nj2kCR/evidence.json`。浏览器配置也全部在脚本创建的 TEMP 目录中；没有读取或修改用户日常浏览器配置，没有使用任何真实 Cookie、登录账号或配对令牌。脚本只为临时扩展写入合成令牌，并在请求后移除。探针打开没有自动桥接请求的扩展设置页，浏览器回显服务绑定 `127.0.0.1` 的随机端口，不占用项目的 `17433`。

**尚未执行**：真实桌面端配对及任务创建、实际公开网站视频的登录后下载、浏览器商店安装版/企业策略、全部历史 Chrome/Edge 版本、无痕交互的所有手动授权配置。真实浏览器探针调用的是扩展页面中的原始辅助模块；无痕 CDP 上下文用于验证 Tab 字段与 store 选择错误，不能替代所有用户无痕设置组合的验收。原生 Rust 本地桥审查中看到的边界另列如下，不把回显服务的成功当成 `/v1` 原生联调成功。

## 2. 已确认的问题

### EXT-01 · P1 · 任意网页可伪造 LanDisk 消息，触发携 Cookie 的原生 POST 任务

定位：[extension/src/landisk-adapter.js:20](../../extension/src/landisk-adapter.js#L20)、[extension/src/background.js:702](../../extension/src/background.js#L702)、[extension/manifest.json:61](../../extension/manifest.json#L61)。

`landisk-adapter.js` 在全部 HTTP/HTTPS 页面注入。隔离世界消息监听只检查 `event.source === window` 和固定字符串 `maobu-landisk-main` / `SEND_LANDISK_BATCH_TASK`，没有验证站点、目标 URL、接口路径或用户操作。网页自身就能提供这些字段；`event.source === window` 无法区分合法适配器与同页面的其他脚本。`isTailscaleOrLanHost()` 仅定义，没有用于这个入口的授权判断。

后台收到消息后，直接取页面/目标 Cookie，然后通过签名桥发送 `method: "POST"`、任意 `body` 的任务，没有重新检查接管开关或 LanDisk 来源。Rust 端认证通过后会把任务交给 manager；POST 探测实际附加任务请求头与请求体，见 [src-tauri/src/manager.rs:3055](../../src-tauri/src/manager.rs#L3055)。

隔离复现结果：公网形态的 `https://unrelated.example/` 页面消息被转发；即使 `intercept: false`，真实后台处理函数仍返回成功，发送载荷含 `POST` 与合成 Cookie。两段探针均使用本地替身，没有对真实站点发送请求。

影响：已配对扩展可能成为网页绕过浏览器 Cookie/SameSite 和交互限制、调用原生 HTTP 客户端的入口，还能在用户关闭接管后创建意外下载。危害取决于目标服务对请求的进一步校验，不能宣称已经攻破任何外部站点。

建议：首先收紧或停用网页可直接调用的通用转发入口；后台必须验证实际 sender、可信站点与严格同源目标、明确的 `/api/download/batch` 路径以及一次性、可核验的用户触发状态。固定 source 字符串或单纯 nonce 广播不能作为页面脚本的可信身份。前置校验失败时保留原生浏览器下载。

### EXT-02 · P1 · Cookie 跨域拼接会泄露页面登录态，并可能串用同名 Cookie

定位：[extension/src/interceptor.js:330](../../extension/src/interceptor.js#L330)、[extension/src/background.js:225](../../extension/src/background.js#L225)、[extension/src/background.js:594](../../extension/src/background.js#L594)。

两个认证头帮助函数都把目标 URL 与页面 URL 的 Cookie 合并成同一条 `Cookie` 头；自动接管还合并 Referrer Cookie。合并按 Cookie 名去重，不保留 domain/path 边界。`findSourceTab()` 找不到确切来源时还会回退到当前活动页，因此这个页面甚至可能与下载无关。

隔离复现：在源主机 `localhost` 放置合成 session Cookie，目标主机改为 `127.0.0.1`；Chrome、Edge 实际 `chrome.cookies` API 都使最终目标请求头包含源主机 Cookie。普通 HTTP 内核会把任务头附给目标请求，参见 [src-tauri/src/manager.rs:3072](../../src-tauri/src/manager.rs#L3072)。

影响：正常的跨域 CDN 下载就可能把源站账号信息传给其他主机；同名 Cookie 冲突还会造成下载失败或身份不一致。该问题发生在网络请求前，后续对日志脱敏并不能解决。

建议：每次请求只选择与该目标 URL、Cookie store 和必要分区匹配的 Cookie，不无条件合并页面或 Referrer Cookie。站点特例需要明确的凭证作用域；重定向时也应重新判断敏感头能否发送。没有确定来源 Tab 时不读取任意活动 Tab 的登录态。为“源域与目标域不同”“同名不同 path/domain”“跨域重定向”增加负向测试。

### EXT-03 · P1 · Chrome/Edge 无痕任务读取正常窗口登录态

定位：[extension/src/interceptor.js:342](../../extension/src/interceptor.js#L342)、[extension/src/background.js:233](../../extension/src/background.js#L233)、[extension/src/background.js:758](../../extension/src/background.js#L758)、[extension/src/popup.js:245](../../extension/src/popup.js#L245)。

代码依赖 `tab.cookieStoreId` 选择 Cookie store，但真实 Chrome/Edge 返回的 Tab 并不提供这个字段。浏览器探针建立正常/无痕两个上下文，分别写入不同合成 session，结果两个浏览器均为 `privateTabHasCookieStoreId: false`、`privateHeaderUsesNormalStore: true`、`privateHeaderUsesPrivateStore: false`。

影响：无痕下载可能认证失败，也可能带上正常账号身份；“支持无痕登录态”这一能力目前不能成立。现有测试手工构造了带 `cookieStoreId` 的 Tab，因此没有暴露真实 API 差异。

建议：在 Chrome/Edge 上按 CookieStore 的 `tabIds` 等实际 API 选择 store，并处理没有访问权限、无痕未授权与无法唯一定位来源的情况；无法可靠选择时明确告知用户，不退回读取其他账号。Chrome 官方说明区分了正常/无痕 cookie store，并提供 `getAllCookieStores()` 与 `CookieStore.tabIds`，参见 [Chrome cookies API](https://developer.chrome.com/docs/extensions/reference/api/cookies)。

### EXT-04 · P2 · 页面接管成功提示中的“撤销”必定失败

定位：[extension/src/content.js:738](../../extension/src/content.js#L738)、[extension/src/background.js:831](../../extension/src/background.js#L831)、[src-tauri/src/bridge.rs:344](../../src-tauri/src/bridge.rs#L344)。

前端“撤销”发送 `action: "cancel"`，后台原样转发；原生桥 `task_action()` 仅处理 `pause` / `resume` / `open_file`，其他动作统一返回“不支持的操作”。这条确定的代码路径与站点、网络无关。

复现步骤：成功接管任意下载后，在页面成功徽章内点击“撤销”。预期取消桌面任务，实际显示撤销失败，任务继续。

建议：补齐协议中 `cancel` 的安全含义及 Rust/扩展联动测试，明确是否保留部分文件；或在实现前移除无法工作的按钮。不能只改成功提示文案假装任务已取消。兼容旧桌面版本时应检测能力并给出对应操作。

### EXT-05 · P2 · 最小文件体积设置被改写，归档和安装程序绕过阈值

定位：[extension/src/background.js:30](../../extension/src/background.js#L30)、[extension/src/background.js:61](../../extension/src/background.js#L61)、[extension/src/interceptor.js:110](../../extension/src/interceptor.js#L110)。

`config()` 将任何 `minSizeMb === 1` 当成旧默认并持久化改成 0，无法区分用户明确设置的 1 MB；桌面端返回 1 MB 时也被转换成 0。此外，`.zip`、`.exe`、`.torrent` 等类型直接豁免体积阈值。

隔离复现：执行真实 `onCreated` 处理函数后，明确写入的 `minSizeMb: 1` 变成 0；用 100 MB 最小阈值评估 10 字节的归档文件，仍得到 `eligible: true`。这与 AGENTS.md 中“接管规则必须尊重文件类型和大小阈值”不一致。

建议：用明确的一次性迁移版本标志处理旧默认，不能按值猜测用户选择；所有文件类型都遵守用户阈值。若产品确实需要“归档总是接管”，应作为清晰、独立、由用户开启的设置并重新确认产品规则。

### EXT-06 · P2 · 磁力发送遇到鉴权失效/服务器错误时不回退原处理程序

定位：[extension/src/background.js:791](../../extension/src/background.js#L791)、[extension/src/content.js:684](../../extension/src/content.js#L684)。

磁力发送只对离线错误返回 `fallback: true`；401、500 等其他错误返回 `fallback: false`。内容脚本已阻止链接默认行为，收到这个结果后仅显示错误，下载没有交还浏览器/系统原磁力处理程序。

隔离复现：合成配对状态下让 `/v1/tasks` 返回 401，真实后台处理结果为 `{ ok: false, fallback: false }`。原有测试覆盖了离线与未配对，但没有覆盖“有过期令牌 → 服务器拒绝”。

建议：在明确没有创建桌面任务时恢复原处理路径；对超时等“不知道服务端是否已接受”的情况另行设计幂等/确认策略，避免盲目重发产生重复任务。补充 401、403、429、500 和鉴权撤销场景。

### EXT-07 · P2 · 嗅探缓存没有随标签导航清空，可能下载上一页媒体

定位：[extension/src/sniffer.js:126](../../extension/src/sniffer.js#L126)、[extension/src/sniffer.js:178](../../extension/src/sniffer.js#L178)、[extension/src/background.js:812](../../extension/src/background.js#L812)。

缓存只按 `tabId` 保存，只有 `tabs.onRemoved` 清理；没有导航时重置或按 document/origin 隔离。查询记录时直接返回该 Tab 全部缓存，不校验当前页面来源。

隔离复现：开启来源站点嗅探并记录一个媒体后，模拟该 Tab 导航到另一站点；不存在导航监听，旧媒体仍可被 `getItems(tabId)` 返回。

影响：扩展弹窗可能显示上一页的链接；多页视频、动态导航或切换站点时，用户可能误选旧视频。此项复现证明缓存边界缺失，尚未逐个实际站点手动重现误下载。

建议：主文档导航时清空；对于 SPA 更新采用可验证的页面/媒体标识更新。关闭站点嗅探后也要清理或明确隐藏已有缓存，并在查询/发送前确认记录仍属于当前页。

## 3. 测试体系缺口（独立于以上 7 项产品问题）

169 个测试全部通过，仍能稳定复现上述问题，说明测试覆盖与断言存在缺口：

- [extension/src/landisk-adapter.test.js:4](../../extension/src/landisk-adapter.test.js#L4) 自己复制域名识别与参数函数，未加载实际适配器的消息桥；因此无法发现真实入口没有使用域名判断。
- Cookie 相关测试把跨域 Cookie 合并当作成功条件；无痕测试提供了实际 Chrome/Edge 不存在的 `tab.cookieStoreId`。测试验证了预设行为，缺少真实 API 合同与跨域负向边界。
- 撤销的前端 `cancel` 动作没有与 Rust action 允许值做联动验证。现有 wiring 测试只证明有同名消息接收方，不能证明动作受支持。
- 磁力测试缺少过期配对令牌/HTTP 错误回退；嗅探测试缺少导航/关闭开关后的记录隔离。

建议保留现有模拟测试，同时把关键消息链路、认证作用域、真实浏览器最小 smoke 纳入回归。新增审计脚本目前用于显式复现和收集证据，**不是已修复后的断言式回归测试**；修复每一项后应将相应观察改成明确的安全行为断言。

## 4. 已检查到的正确边界与待补验项

本地桥监听明确为 `127.0.0.1:17433`，没有开放通配 CORS；配对入口有独立限流、一次性配对码，正常业务校验扩展 Origin、时间戳、扩展标识和 HMAC。JS HMAC 与既有协议算法一致，真实 Chrome/Edge 发出的 GET/POST 也保留对应扩展 Origin。这些已有控制应保留，修复 EXT-01 时重点是扩展自身的网页信任边界。

普通 HTTP 接管具有发送失败后恢复浏览器下载、暂停前持久化 pending 标记、Service Worker 重启看门狗，以及 `task-sent` 后避免恢复重复下载的分支。现有模拟测试通过。**未实测整个浏览器进程崩溃/强制结束与原生桌面任务同时运行的恢复**，不能仅凭单测宣布崩溃恢复完整通过。

站点方面，已检查扩展存在通用语义元素、媒体嗅探、页面 URL 解析，以及 PikPak/LanDisk 特殊分支；B 站/抖音等公开站点的实际格式、登录态、音画合并、时效 URL 和下载完整性需要结合媒体模块报告逐站验收。这份扩展报告不宣称“全部站点均下载成功”。

## 5. 建议修复次序与兼容要求

1. 先修复 EXT-01 网页到原生请求的信任边界、EXT-02 Cookie 作用域、EXT-03 无痕账号隔离。
2. 再修复 EXT-04 撤销协议、EXT-06 磁力错误回退，补上真正可用的交互闭环。
3. 然后修复 EXT-05 用户规则、EXT-07 页面媒体缓存，并建立上述负向回归。

应保持现有端口、`/v1` 协议头、已有命令名与旧版本请求默认值。`cancel` 属于新增支持的动作，需要 Rust/扩展同步测试及旧桌面兼容策略；不需要为这些问题更换端口、重置配对数据库或删除用户下载。当前报告中的缺陷均仍待生产代码修复，不能把本轮审计脚本的执行成功写成产品问题已修复。
