# 真实浏览器扩展 → Rust 本地桥隔离验收

日期：2026-10-31。实际 Edge 156.0.4314.50、Chrome 154.0.8037.92，均使用独立 headless 临时配置。不是 fetch 替身、Node 回声服务或手工重写 HMAC 代替产品实现。

## 隔离与真实路径

- 复用已有单 Wry 测试上下文与独立 Manager/SQLite/下载目录；不创建第二个 Wry，不管理真实 AppData。
- `bridge.rs` 仅抽取原 Router 构造。生产仍只监听 `127.0.0.1:17433`；测试绑定 `127.0.0.1:0`，所有真实路由、Origin/HMAC/时间戳、配对码、撤销、限流与管理器动作保持原代码。
- 临时复制扩展 manifest/src，仅副本 `protocol.js` 的 API 地址换成随机测试端口。正式源端口没有修改，用户已运行的 17433 客户端未停止、未发送测试任务，也没有向日常浏览器安装扩展。
- 正向配对调用实际后台 `chrome.runtime.sendMessage({type:"pair"})`；请求使用生产 `signedFetch/signedGet`，暂停/恢复/取消使用生产 `performBridgeTaskAction`。Cookie 来自临时配置的真实 Chrome API 合成身份。
- 浏览器下载目录被明确指定到该测试的 TEMP。令牌与一次配对码不写日志；测试阶段文件不提供额外 HTTP 控制端点。

## 实际结果

命令：同一已构建测试 EXE，以 `MAOBU_FETCH_ISOLATED_TEST=1`、`MAOBU_TEST_BRIDGE_BROWSER=1`、`MAOBU_TEST_BRIDGE_BROWSER_ONLY=1` 运行原生聚合入口。

**1 通过、0 失败、0 忽略，5.39s**。每个浏览器均通过：

| 场景 | 实际结果 |
| --- | --- |
| 实际后台配对 | 成功，令牌仅存临时扩展存储。 |
| 错误配对码 / 已消费配对码重放 | 均 401。 |
| 扩展标识与实际 Origin 不一致 | 403。 |
| 过期时间戳 / 签名正文被改动 | 均 401。 |
| 新建暂停任务 → 恢复 → 暂停 → 真正取消 | Rust 状态及活动连接归零正确；取消没有最终文件，没有降级假称取消。 |
| 新建任务 → 下载完成 | 实际原生下载，Store source=browser；认证头清除；最终完整 SHA 与本地源一致。 |
| 桌面端撤销配对 | 401，扩展二次确认 health 后清除存储令牌。 |
| 连续未授权请求 | 实际 Rust 限流返回 429。 |
| 桥离线后的浏览器下载回退 | Rust 主动停止该临时服务器，再执行生产右键下载处理；真实 `chrome.downloads` 完成，文件在 TEMP，通知文案真实报告交回浏览器。 |

每个浏览器的 Rust 完成文件、离线回退浏览器文件和源均为 **8,388,745 字节**，SHA-256：

`18cb64a3cb1fefe667e560b4eb76fe11b6002b54ce961bb7f729b93cdf13a9bb`

证据：`%TEMP%/maobu-real-bridge-browser.log`。可复现脚本：`scripts/audit/bridge-browser-e2e.mjs`、`bridge-auth-checks.mjs`、`bridge-task-checks.mjs`；Rust 为 `bridge/browser_fixture.rs` 与 `manager/production_http_tests/bridge_cases.rs`。

## 全套回归中的夹具修正

最终全套第三轮曾在等待 Downloading 时失败；同版独跑4.51秒通过，同版完整原生聚合90.47秒也通过。为取证，在恢复后明确延迟3秒再读实际任务，得到 completed、progress=1，而非网络错误。原因是桥夹具使用 Good 响应，设置 slow=true 仅对 SilentBody/NoValidatorBody 生效，任务实际并未停顿，快速完整下载会越过中间状态。

桥专用夹具改为真实 SilentBody，并将其连接超时由2秒设为30秒，保证大于浏览器12秒状态等待且小于源60秒停顿；不修改产品默认设置、12秒等待、120秒总期限或暂停/取消断言。证据 `%TEMP%/maobu-bridge-delay-proof.log`、`%TEMP%/maobu-native-bridge-diagnostic.log`。此夹具修正后的最终统一回归结果由主报告登记，本节不把此前通过计数冒充修正后已执行。

正式 release 的隔离 smoke 另加仅生产 run 且 `MAOBU_FETCH_ISOLATED_TEST=1` 生效的安全层：GET health保留真实版本、禁用接管/磁力标记，其他方法和路径在认证或配对副作用前统一503，避免临时空配对Store向用户已配对扩展返回401。该层不施加于随机端口的测试Router。真实HTTP合成令牌测试与扩展真实503保留令牌回归已落，后者已实际通过；没有读取或复制用户令牌。

## 验证边界

这是实际产品协议/浏览器/原生管理器的隔离联动。创建载荷使用生产签名器与真实 NewTaskRequest；此场景没有触发网页普通下载的自动接管拦截流程，该流程另有模拟 Chrome API、Cookie 隔离与 LanDisk 双浏览器真实确认测试。没有执行系统文件打开、真实账号网站、商店分发或 WebView2/企业策略版本矩阵。

正式 release EXE 的 17433 health、包内清单、资源属性和安装升级由发布检查单独验收；该测试不能替代这些检查。
