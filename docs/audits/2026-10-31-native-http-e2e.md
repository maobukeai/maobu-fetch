# 原生下载管理器 HTTP 集成复验

日期：2026-10-31（Asia/Shanghai）。用例：`manager::production_http_tests::native_manager_http_downloads_are_integral_resumable_and_cancellable`。

后续补入公网发现的 source/codec 修复后，最新完整 round10 **1471 passed / 0 failed / 16 ignored，97.61 秒**，另 **9 项 Range 全通过，5.07 秒**。274 个 Rust 源文件快照 Changed=0。下文早期各轮结果保留；两条抖音的真实队列整片 4/4 证据另见 `2026-10-31-douyin-source-fixes.md`。

统一复验 `TEMP/maobu-final-cargo-round6.log`：**1429 passed / 0 failed / 16 ignored，113.50 秒**；另 9 项 Range 回归 **9 passed / 0 failed，5.06 秒**。真实 manager 默认聚合已运行完成，且该轮显式开启 `MAOBU_TEST_EXTERNAL_MEDIA=1`、`MAOBU_TEST_BRIDGE_BROWSER=1`，末尾真实工具与 Chrome/Edge→Rust 桥组也实际执行通过；M05 普通暂停/恢复/显式停录及首段无 body 的失败收尾均在默认聚合中运行。设置持久化兼容修复后的最终 round7 再次完整通过：**1432 passed / 0 failed / 16 ignored，111.78 秒**，另 **9 项 Range 全通过，5.07 秒**。下文保留早期失败和修复记录，不把历史失败或单独 selector 混成最终结果。

## 隔离与真实调用范围

单一 Windows Wry context，配置先移除全部窗口、使用唯一测试 identifier，不运行可见桌面 UI。基础 Harness 不注册生产本地桥、单实例插件、自动启动或调度循环；显式浏览器 opt-in 组另开真实 Rust 路由与受控调度，桥只绑定随机本机端口并明确禁止 17433。使用临时目录中的真实 SQLite Store，关闭通知和自动更新；实际 BT 下载、恢复与做种样本也限定在临时数据目录和本机受控服务。测试结束停止下载；不读取个人数据库或 Cookie，不请求公网。

用例从临时 Store 中的 Queued 任务调用真实 `dispatch_once`，进入 `spawn_worker`、`run_worker_loop`、`download_once`、Range/普通流、合并、校验和状态持久化。暂停、恢复和退出走真实 manager action/shutdown；续传重新打开同一临时数据库并调用实际启动恢复。测试没有复制 HTTP 下载实现，未覆盖 NewTask 前端表单或桥接创建任务 API。

本地 axum fixture 使用随机 127.0.0.1 端口和 8 MiB + 137 字节确定性源数据，记录实际收到的 Range；普通完成同时核验文件实际字节长度和 SHA-256。重启场景模拟已持久化的中断状态后重新构造 manager，属于数据库/内核重建验证，不能等同于强杀进程后恢复。

## 覆盖清单

| 场景 | 关键断言 |
| --- | --- |
| 1 / 4 / 8 连接各一次 | 逻辑分片数准确；服务端有界屏障验证真实并发连接峰值恰好为 1/4/8；剔除 0-0 探针后，实际请求完整覆盖且不重叠、不重复；最终长度与 SHA-256 一致 |
| 忽略 Range | 安全单连接回退，直接使用 200 探针 body，不重复下载全量实体 |
| 未知长度 | 单连接完整收尾，最终真实总长和 SHA-256 正确 |
| 错误 Content-Range | Failed，不生成目标文件，活动连接归零 |
| 416 | 请求次数受既有空响应熔断上限约束，不能无限重试或标为完成 |
| 静默响应头 / 静默 body 各暂停一次 | 2 秒内停止；返回后任务数据指纹保持稳定；恢复后输出正确 |
| 静默响应头 / 静默 body 各总超时一次 | 1 秒任务总期限，4 秒内终态失败；所有真实连接退出，停止后不继续写入 |
| 连续四次暂停恢复 | 每代均等待旧 worker 退出；不增加并行 writer；最终 SHA-256 正确 |
| 临时 SQLite 重新打开续传 | 启动恢复不改分片字节；请求从持久化分片的精确下一字节继续 |
| ETag 大小写变化 | 保留原验证器与所有旧分片指纹，RemoteChanged，不能拼成目标文件 |
| 已完整临时文件在发布前重启 | 原验证器确认后直接收尾，不发 `bytes=total-` 导致伪 416 |
| 最终发布失败 | 目标路径被占用时完整临时文件及全部源分片保持，释放后恢复并核验 SHA-256 |
| 等待网络期间总超时 | 总时限包含网络等待，不能停留在无界等待状态 |
| 原生 BT 生命周期与后台做种 | 复用同一个 Wry handle，在隔离目录中调用真实 BT 生命周期用例；并发 1 时背景 seed 不占用 HTTP 任务槽 |
| 五盘原始分享页 | 明确失败并要求先选文件，没有 HTML 假完成、自动子任务或父任务删除后的延迟子任务 |
| HTTP 与 BT 真实混合带宽 | 独立 BT 带宽模块测实际网络/已校验字节，不能仅用 limiter 设置值冒充吞吐证据 |

## 执行记录

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib native_manager_http_downloads_are_integral_resumable_and_cancellable -- --nocapture --test-threads=1
```

历史基线结果：**1 passed / 0 failed / 0 ignored，最初 14 个实际场景全部执行通过**，该轮包含并发峰值严格校验，运行 10.36 秒。此后增加重启发布、源分片保留、网络等待、BT 和网盘边界等场景，并继续使用同一个真实聚合测试；新增覆盖的最终整轮结果需以本节后续执行记录为准，不能将历史 14 项通过扩写成最新全部通过。测试程序由 Cargo 正常编译链接后直接运行，没有手工修改 EXE。

可选的 `MAOBU_TEST_EXTERNAL_MEDIA=1` 在聚合末尾调用独立临时 manager，执行机器上已安装的 yt-dlp/FFmpeg 本地媒体用例，缺工具明确输出 Skipped。诊断 selector `MAOBU_TEST_EXTERNAL_MEDIA_ONLY=1` 必须同时设置前述显式 opt-in；`MAOBU_TEST_NATIVE_ONLY=bt-bandwidth` 仅运行 BT 吞吐组。这些 selector 用于定位前置失败，不替代默认完整聚合回归，也不能把未执行的组计为通过。没有创建第二个 Wry 事件循环或访问个人工具配置。

多连接实现已将原 918 行 `download_segments` 拆为私有签名兼容的短入口，以及 plan/runtime、reporter/disk、workers、window/response/retry、outcome/merge 小模块。任务子工作仍全部跟踪并等待退出；文件只在完整合并、长度复核、同步并关闭句柄后原子替换临时成品。任何分片退出都严格处理 flush 结果；合并复制支持取消，失败不删除源分片。另有实际文件系统回归核验合并字节/SHA、坏片/缺窗口/总长不符和复制中取消后的数据保留。

最新核心诊断执行：使用 Cargo 正常重链的同一测试 EXE，设置 `MAOBU_TEST_NATIVE_ONLY=http-core`，**1 passed / 0 failed / 0 ignored，20.56 秒**。实际覆盖创建/原子命名、上述 17 个 HTTP 场景、晚到额外 chunk 的拒绝与隔离重启、五个验证器/协议子场景，以及五盘分享页边界。验证器场景包含旧任务 ETag/Last-Modified 为空但新探针有验证器的 4 连接和 1 连接，以及数据库进度为 0、磁盘仍有字节的分片任务；全部必须 RemoteChanged 并保持每个旧文件的长度与 SHA。

晚到额外 chunk 已先负向复现：服务器给满合法 Range 字节后延迟 25 ms 追加 17 字节，旧生产实现错误得到 Completed。修复后读取至 EOF 并严格核对声明长度；坏窗口 flush、关闭后隔离为 `.rejected-UUID`，先落盘阻止续传的标记再移动，重启不能凭其完整长度复用。真实恢复/重试仍拒绝，原隔离证据字节保持。

进一步补上写满字节但尚未读到 EOF 时崩溃的窗口：每次打开分片写入前先同步持久化 `pending-validation` 身份记录，只有请求完整结束、flush、sync、关闭文件后才能移除。重启遇到满文件和残留记录，必须隔离保留后重新取该 Range；部分文件必须有可用于 If-Range 的身份才能继续，异常记录/链接/标记 IO 失败均明确失败。新增真实 manager checkpoint 用例构造“满窗口＋残记录”，重新打开数据库和 manager 后要求实际再次发出该完整 Range、最终长度/SHA 正确。这是持久化 checkpoint 恢复验证，不冒充操作系统强杀或断电实测；该新增项随最终全套执行。

同一新 EXE 的 `manager::http_segments::` **5/5 通过**，包含合并字节/SHA及数据保留、坏片/缺窗口/总长不符、真实 64 MiB 复制中取消、超长 legacy/window 文件不自动删除、非普通文件与目录枚举错误明确失败。新的通用逐跳 HTTP 请求器另有真实本地重定向回归，首次三项已通过；后续显式 Basic Auth 兼容场景纳入最终全量测试。

完整默认聚合在上述核心拆分后实际执行到 BT mixed warm-up；该轮因 BT 预算热身问题失败，未执行末尾工具组。此后 BT 负责人已独立复验修正后的混合带宽组，媒体负责人也独立复验工具组；最终默认聚合仍须统一重跑，独立通过不能改写为该轮完整成功。根任务新增发布与数据库完成事务之间的 journal 恢复后，另有真实 SQLite trigger 故障用例（1/4 连接）进入默认聚合，最终结果待统一记录。

正式全量 Cargo 后续当轮结果：**1412 passed / 2 failed / 16 ignored**。默认真实 manager 聚合已实际执行并通过：创建流程、1/4 连接 SQLite 完成提交故障及源服务关闭后的离线 journal 恢复、全部核心 HTTP 场景、晚到额外 chunk、满 pending checkpoint 重启重取，以及三种旧验证器缺失场景；均保留原完整性断言。该轮聚合继续到 BT mixed 实际限速后，暂停时事件进度与 fixture 磁盘计数相差 410 字节而失败，后面的组没有执行。另一个失败是原请求头单测用了无 `name=value` 的非法 Cookie fixture；根任务修正 fixture，保持生产解析严格。本专项的文件系统 5 项、通用 HTTP 逐跳 4 项、Cloud 逐跳/代理 10 项及分页 9 项均在该轮通过。

该轮新增 EOF 崩溃窗口与数据库完成故障已获得真实 manager 行为证据，但**当时整套仍有上述两个失败，不能标记全绿**。BT 负责人已实证该 410 字节恰好是四个 pending-validation JSON（98/104/104/104 字节），属于 fixture 将元数据误算为下载 payload，并非生产 BT 丢失进度；已收紧为精确分片文件名并新增 sidecar 回归，原事件/落盘相等断言保留。修正后 round5 已完整通过，补入停录首段无数据的原子收尾后按本报告开头 round6 再次统一复验通过，未以核心 selector 代替。

本轮同时发现并修复测试构建问题：Wry 实际构造新引入 Common Controls v6 导入，Windows 测试 EXE 起初因缺少 v6 manifest 在测试函数之前以 `0xC0000139 / STATUS_ENTRYPOINT_NOT_FOUND` 退出。根任务将正确 manifest 的嵌入纳入通用 Windows 链接参数，移除重复 RC manifest 资源，随后 Cargo 重新链接的测试产物可以原生启动并运行全部场景。最初的加载失败没有被跳过、ignore 或用伪 AppHandle 替代。

## 验证边界

本用例不覆盖真实 CDN、代理认证、TLS 握手失败、磁盘满、Windows 休眠/唤醒、进程强杀、输出被其他软件锁定后的最终发布、浏览器扩展交互或完整桌面视觉验收。独立 HLS 引擎探针另有 37 项已运行的生产源码及 Cookie scope 回归，见 `2026-10-31-m3u8-fixes.md`；两者不得混为同一层级的证据。
