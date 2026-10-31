# M08-Cloud：网盘请求出口、取消与分享页任务边界

日期：2026-10-31（Asia/Shanghai）。范围：百度网盘、夸克、PikPak、123 云盘、蓝奏云的分享解析和直链获取；与 M12 分页修复分别验收。

## 已确认的问题与修改

原五盘实现使用独立 Client/静态客户端，部分后备直链请求又新建 no-redirect Client。应用手动代理、任务代理和代理认证没有贯穿分享页、提取码、验证码、目录、转存、轮询与最终地址提取；下载实体与前置解析可能使用不同出口。取消任务也不能及时打断这些前置请求。

新增 `CloudRequestContext`，一次注入相同代理配置的普通客户端和禁止重定向客户端，以及同一 CancellationToken。五盘全部生产阶段共用该上下文；平台原有 User-Agent 保留。百度 PCS 与蓝奏最终 302 提取仍禁止跟随重定向，但继续使用相同代理与认证。上下文将整个操作限制为 120 秒，响应最多 4 MiB；HTTP 错误和无效正文明确失败。任务取消会丢弃正在等待响应头、读取正文或轮询的 future。

随后收紧 Cookie：小型 CloudRequestBuilder 保存原始 jar 和逻辑 URL，使用禁止自动重定向的同一客户端逐跳发送，最多 10 跳；每跳在测试端点映射前根据真正目标的域、path、Secure 和过期时间生成 Cookie。平铺 Cookie 仅绑定指定的原 API origin，不能因同一平台后缀相似就发给 PC/PCS/CDN；跨域认证需要具备相应域作用域的 jar。分页也复用该请求器，避免裸 Client 或自动重定向沿用已经扁平化的 Cookie。自定义认证头在跨 origin 跳转时按白名单剥离；不可克隆的流式正文明确拒绝，POST 重定向遵守 301/302/303 与 307/308 的不同规则。

百度动态 BDCLND 追加为仅原来源域可用的 jar 条目，保留其他 Cookie 的域与路径。123 的 jar 不再被误识别成一整段 Bearer；只有对真实 API 地址有效的 token Cookie 才可提取成认证头。蓝奏 WAF Cookie 也只归属原分享源，iframe/CDN 换域不能盲目继承。PikPak 的设备签名/captcha 请求共用该逐跳安全规则。

根任务接入统一的应用/任务客户端工厂，桌面命令、下载前解析、直链刷新和媒体 PikPak 分支均调用 `*_with_context`。任务显式代理优先于应用设置；显式空代理表示直连。上次下载的内网 CDN 地址不能使后续公共网盘 API 意外绕过应用代理。旧公开签名保留为兼容 wrapper；实际产品路径通过注入入口获取设置和取消信号。没有修改环境代理或新增依赖。

平台错误不再拼接原始 JSON 正文、远端 message 或完整临时地址；底层请求错误移除 URL，统一上下文再做凭据脱敏与地址隐藏。客户端构造错误仍会使操作失败，不能悄悄换出口。

原 manager 自动展开分享页只实现三盘，另两盘可能把 HTML 当普通文件；展开采用未跟踪的无界 spawn，父任务删除后仍可创建子任务，子文件取链失败还被吞掉。现使用 URL 的 host/path 精确识别五盘分享页，明确失败并引导先解析、勾选文件，不自动产生子任务。已解析 CDN、API/PCS 地址、普通文件、POST 下载、相似域名以及只在 query 中出现分享地址的 URL 不被泛字符串匹配误拦。此项有意改变旧的分享页自动展开行为，保证用户选择的文件身份与任务范围。

## 实际执行证据

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib cloud_request::tests -- --nocapture
```

最初结果：**5 passed / 0 failed / 0 ignored**，0.53 秒。逐跳 jar 修复后的实际结果：**8 passed / 0 failed / 0 ignored**，0.54 秒，保留全部原用例并新增三项实际作用域测试。使用实际 AppSettings/任务客户端工厂、真实 reqwest、五盘生产入口和随机本地 HTTP 代理；代理要求合成 Basic 认证，并记录收到的请求路径，不输出查询参数或凭据。

| 测试 | 实际断言 |
| --- | --- |
| 五盘完整解析与取链 | 百度提取码/分享 HTML/目录/取链、夸克 token/目录/PC 取链、PikPak captcha/目录/取链、123 目录/取链、蓝奏 WAF Cookie/iframe/JS/AJAX/302，全部抵达指定认证代理 |
| 后备路径 | 百度主路径失败后转存与 PCS no-redirect、夸克限制错误后的转存及任务轮询继续携带代理认证 |
| 认证失败 | 错误代理密码收到 407，操作失败、没有成功 origin 路径或静默切换出口，错误不包含测试密码 |
| 取消 | 五盘分别在静默响应头与静默 body 阶段取消，共 10 个子场景，均 500 ms 内返回取消错误 |
| 工厂优先级 | 真实应用代理、任务代理及任务认证覆盖、显式直连、旧内网 CDN 不绕过公共 API 代理；直连 origin 断言没有 Proxy-Authorization |
| 真实 Quark inspect 逐跳作用域 | 相对跳转、同 host 不同 path、跨 host，再接目录分页；jar 各跳只发送匹配条目，过期/外域条目不发；平铺 Cookie 同源保留、跨源清除 |
| 实际 PCS / PC 域 | 百度转存后 PCS 仅收到自己的 jar 条目；BDCLND 保留于原百度域。Quark PC 仅收到 PC jar 条目，平铺 API Cookie 不转发到 PC host |
| 123 jar | 合法域/path 的 token Cookie 正确形成 Cookie 与 Bearer，未把原多行 jar 当作认证头 |

后续正式全套中，本模块 **10 项全部执行通过**，包括新增的 HTTPS→HTTP 逻辑跳转中 Secure Cookie/Authorization/设备签名过滤，以及 CloudRequestBuilder 的五类 POST 重定向精确正文测试。历史当轮合计 **1412 passed / 2 failed / 16 ignored**；失败来自原请求头测试的非法 Cookie fixture 和混合带宽 fixture 把分片验证记录计入 payload，当时没有据本模块通过把整套写成全绿。修正 fixture 并保留原断言后 round6 已统一通过；设置持久化兼容修复后的 round7 为 **1432 passed / 0 failed / 16 ignored**，另 9 项 Range 回归全部通过。后续公网适配修复后的最新完整 round10 为 **1471 passed / 0 failed / 16 ignored**，另 9 项 Range 全通过；该增量索引不改写前述历史。

端点仅在 cfg(test) 下映射到合成 origin，由本地代理直接回包，不连接真实平台，不读取个人 Cookie、数据库或账号。用例验证生产解析及客户端路由，不复制网盘请求逻辑。

分享页 guard 另有纯 URL 分类回归和真实 manager 聚合场景：五盘父任务明确失败、原 URL/文件名保持、没有输出或子任务，删除后仍无延迟子任务。聚合用例的最终运行结果统一记录在 `2026-10-31-native-http-e2e.md`。

## 边界

认证代理 fixture 使用 HTTP 合成 origin，尚未覆盖真实代理的 HTTPS CONNECT、TLS 拦截、系统 PAC、SOCKS 或平台当前风控。没有使用用户自有分享做线上取链验收；这些平台的私有协议变化仍需合法样本单独验证。通过本地代理测试不代表全部真实平台已经在线可用。
