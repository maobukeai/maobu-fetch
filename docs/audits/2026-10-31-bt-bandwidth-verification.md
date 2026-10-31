# HTTP/BT 实际吞吐与做种生命周期验收

日期：2026-10-31。只使用独立临时数据库、下载目录、loopback HTTP Tracker/Range 服务和进程内原生 librqbit Session；没有启动正式用户客户端，没有读取用户任务、Cookie 或私人种子。

## 实际上传：通过

同一真实 manager 先把预置的本地完整文件交给生产 BT worker 做分片哈希校验，完成后确认默认关闭做种。随后显式开启做种，让另一个真实原生 Session 下载 32MiB+137 字节。原始 fixture seed 的上传统计最终为 0，因此 leecher 收到的内容来自受测应用任务。

| 设置与窗口 | 实际已校验 payload | 实测速度 |
| --- | --- | --- |
| 单任务 64KiB/s、全局 128KiB/s；6.013 秒 | 393,216 字节 | 65,389 B/s |
| 单任务 0、全局 128KiB/s；6.010 秒 | 786,432 字节 | 130,844 B/s |
| 全局 0；显式取消后恢复，6.206 秒 | 30,064,640 字节 | 4,843,984 B/s |

低速窗口先预热 1 秒再测 6 秒，判定包络为目标值的 50%–120% 加 64KiB，包含完整分片才计进度及在途数据的误差。实际两个低速窗口均接近设定值，而非仅查询 limiter 配置。

同时实际验证：全局做种开关关闭后，drain 500ms 后的 1 秒没有新 payload；全局 0 时在尚未完成的传输中取消，drain 后数据稳定，原任务仍为 Completed 且 seeding_requested=false；再次显式请求后完成同一 leecher 文件，长度和 SHA-256 与源一致。做种停止没有把已完成任务改为暂停或删除记录。

执行测试 EXE 的单 Wry 场景，环境为 `MAOBU_TEST_NATIVE_ONLY=bt-bandwidth`、`MAOBU_TEST_BT_RATE_KIND=upload`。结果：**1 passed / 0 failed**，约 29.47 秒。日志：`%TEMP%/maobu-bt-bandwidth-upload.log`。

## 同时 HTTP Range + BT 下载：通过

首轮真实观测到 BT 有 1 个 peer、约 3.34MiB 已校验进度，HTTP 已接受约 2.64MiB payload。最初用下载文件长度作为暖机条件，四个默认 1MiB BufWriter 尚未 flush，导致文件仍为 0 而测试超时。这个失败不能据此报告为下载停止或限速失效。

测试已改为监听生产 HTTP 每次客户端 write_all 成功后累加的 progress 事件；每个采样端必须新于 500ms，不能假定事件始终严格按 250ms 到达。HTTP 额外误差 128KiB，BT 完整分片误差 32KiB，合计 160KiB。暂停等待所有 writer flush 后，事件总字节必须等于实际分片文件总长度和持久化进度，再核验稳定和最终完整文件 SHA-256。没有使用服务器写入量推定客户端已接收量。

在更新后的真实测试 EXE 实跑通过：

| 同一 6.014 秒窗口 | 实际 payload | 实测速度 | 对应预算 |
| --- | --- | --- | --- |
| 4 个真实 HTTP Range 连接 | 1,400,832 字节 | 232,914 B/s | 262,144 B/s |
| 真实 BT 已哈希分片 | 1,572,864 字节 | 261,517 B/s | 262,144 B/s |
| 两者合计 | 2,973,696 字节 | 494,431 B/s | 全局 524,288 B/s |

实际 Range 峰值为 4。主体包络为每项预算的 50%–120%，再加上述有界采样/分片误差。暂停后真实 flush 长度、事件累计字节与数据库进度完全相等，继续等待 350ms 后没有写入。把全局限速设为 0 并恢复两个原任务，剩余 38,003,615 字节在约 2.066 秒完成，约 18,389,686 B/s；两个完整文件长度、SHA-256 与源一致，两个任务记录保留。

环境为 `MAOBU_TEST_NATIVE_ONLY=bt-bandwidth`、`MAOBU_TEST_BT_RATE_KIND=mixed`。结果：**1 passed / 0 failed**，约 14.51 秒。日志：`%TEMP%/maobu-bt-bandwidth-mixed.log`。旧文件长度计量失败轮次没有算作成功。

## 清空历史与原生做种记录

真实 requested seed 后调用 manager.clear_history(true) 的 fixture 已随全量 round2/round3 的单 Wry 场景实际执行通过：Completed BT 与 Cancelled HTTP 记录清除、原生 Session 中无 torrent、后台 monitor 超过 2.2 秒不复活，下载内容、HTTP 已有文件和无关哨兵 SHA-256 保持。round3 的 NATIVE_BT_HISTORY 四项结果全部为 true。

同时补上做种重检与删除记录之间的并发保护：BtRemovalGuard 持有 seed_sync，调用者一直保留到记录删除/归档/重新下载状态提交，锁序为任务操作→做种协调→torrent 注册。独立 native 删除兼容入口也取同一租约；做种协调内部发现记录已删除时使用不重入协调锁的清理函数，移除刚准备的 paused binding。新增租约和真实原生 paused torrent 清理两项测试已在 round2 通过。

## 最后发现的重检取消挂起

原生暂停完整性重检后可保持 Initializing{paused:true}；原 wait_until_initialized 会持续等待该状态。用户在重检中停止做种后，如果协调锁仍由旧等待持有，后续停止/删除可能等待不结束。

新增当前 preparation 的任务身份与取消 token。外部暂停/显式停止、关闭全局做种、退出和删除等待都取消该 preparation；内部自身暂停使用独立入口避免自取消。取消只暂停并释放协调，不把取消记成真实失败，不写回 requested=true，不自动重排删除任务。真正失败也只对仍为 Completed 且仍请求做种的当前数据库行原子写错误。再次明确准备做种时，遇到已取消的 paused Initializing，保留实际文件、按同身份重绑 paused native 元数据并重新校验，避免调用会打开下载 peer 流的 unpause。

四个确定阻塞测试使用真实已哈希 4 字节种子、current-thread 调度使原生状态精确停在 paused Initializing，然后测试真实等待的取消与锁释放。**round3 四项全部通过**，覆盖 pause+再次完整校验、false 意图+晚到错误/删除不复活、删除租约取消、global-off 与 shutdown。该组 whole shutdown 的初始 1 秒总界曾未考虑 librqbit 已有的固定 1 秒 grace，现拆为 preparation 自身 1 秒取消界与包含 grace 的 3 秒完整退出界；没有改变生产退出时长，也没有放宽 pause/delete/false 的取消界。

round2 前还修正测量 helper：新增 pending-validation sidecar 的 4 份 JSON 长度合计正好 410 字节，原 starts_with 计数误把它们当 payload。现在只计精确的纯数字 legacy part 与数字 window 文件，并新增排除 sidecar/rejected 的回归；暂停的字节完全相等断言没有放宽，round2 已通过。

## 最新统一回归记录

round3 BT 阶段实际测量：混合 HTTP 1,269,760 字节、BT 1,589,248 字节，同一 6.005 秒合计 476,104 B/s，符合全局 524,288 B/s 包络；全局 0 时剩余 37,610,399 字节约 1.028 秒完成。上传两低速窗口实际 65,432 / 130,773 B/s；0 限速取消后恢复约 6.231 秒收到 30,179,328 字节，约 4,842,985 B/s。所有暂停严格字节相等、完整 SHA-256、做种开关和历史清理断言均已执行。

统一日志：`%TEMP%/maobu-final-cargo-round3.log`。该轮 **1,420 passed / 1 failed / 16 ignored**，约 102.56 秒；唯一失败发生在所有 BT 和媒体阶段完成后的浏览器桥场景，等待任务进入 Downloading 超时，由对应分工继续处理。本报告的 BT 回归已通过，不能据此把整个项目的最终门禁写成全绿。

## 边界

本地 loopback swarm 能隔离和重复验证真实生产 manager 调度、全局与单任务 limiter、数据校验和做种开关；它不能替代公网 Tracker/peer 互通、NAT、IPv6 或长时间运行验收。单独的官方 Ubuntu 受限公网协议探针见 `2026-10-31-bt-public-probe.md`，该探针也没有冒充生产引擎公网传输测试。
