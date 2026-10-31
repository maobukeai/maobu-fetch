# 恢复数据归属、做种删除与并发状态交叉复核

日期：2026-10-31。只读复核生产 temp_layout、part_cleanup、startup_recovery、history 和 Store 状态事务；本分工只新增获授权的回归 fixture，生产接线由主修复分工完成。

## 发现与对应修改

| 问题路径 | 影响 | 当前修复方式 |
| --- | --- | --- |
| 旧 flat 分片收养后，任务最终名变化；唯一归属只比较当前 file_name | 第二个旧任务仍叫原名时可能错误收养/清理第一任务的原分片 | 同时读取其它任务的恢复 marker，按原 source_name 判断已认领源；旧同名有歧义时拒绝，保留文件 |
| 启动恢复按 private 目录是否存在决定回落 flat | 新任务虽有明确命名意图，也可能把同名旧 flat 数据计成自身进度 | 通过统一 ensure_owned 判断，保留新任务 intent 与旧唯一归属规则 |
| 已存在 marker 的恢复分支跳过临时目录祖先校验 | 暂停后把 taskdir 换成 junction，匹配 marker 可让续传跨出任务目录 | resolve 和完成发布恢复先核验 root/taskdir 非 reparse，marker 本身也拒绝 redirected |
| 完成发布 journal 的存在性检查提前返回 | junction 内的 journal 可能跳过启动路径验证 | HTTP 恢复在该分支前验证祖先，遇到异常保留数据并明确拒绝 |
| worker 状态整行提交与用户选项、reorder 并发 | 旧快照可能覆盖新的限速、重试、代理或排序 | Store 事务只更新已有行，保留用户选项及 queue_position；失败与 panic 收尾也应走同一事务 |
| Queued rename 的读状态、写整行与 dispatcher 未串行 | dispatcher 可在读后启动任务，rename 再把旧 Queued 写回 | 同一 task_operation 锁覆盖重命名校验与提交 |
| 做种重检与 native 移除、DB 删除未覆盖同一串行区间 | 删记录前后的窄窗口可能留下新 paused native binding | BtRemovalGuard 从 native 移除持有到记录提交；做种重检后发现 DB 缺失时立即清理新 binding |

这些是从实际代码可达分支确认的逻辑问题；不把未执行的竞争窗口或 junction 场景写成已发生的用户数据事件。

## 新增行为 fixture

- temp_layout 的真实 Windows junction 测试只在独立 TEMP 下创建目标、匹配 layout/journal、下载样本与哨兵。验证 resolve、resume、已有字节检测与发布恢复拒绝，启动检查明确失败，目标全部字节不变；用 RemoveDirectory 只移除 junction 本身并再次确认目标存活。
- 旧源已经被 A 收养、A 最终名称变化后，B 使用旧原名必须拒绝借用与删除，原分片不变。
- BT 移除租约测试验证 native 调用结束后，调用者的 DB 提交窗口仍排除做种协调；真实 paused torrent 在已经持有协调锁时能清理而不死锁。
- 同一真实 manager 上传场景末尾验证 requested seed → clear_history(true)：BT/Cancelled HTTP 记录清空、native torrent 数为 0、下载内容和无关哨兵 SHA-256 不变，超过 monitor 周期后不复活。

全量 round2/round3 中上述真实 junction、旧 marker 重命名、directory alias、租约及 clear_history fixture 均实际执行通过；没有操作用户真实目录。round3 的重检取消四项测试也全部通过；该轮总计 1,420 passed / 1 failed / 16 ignored，唯一失败已转为后续浏览器桥场景，不能称整套全绿。已实际完成的混合下载/上传与重检取消细节另见 `2026-10-31-bt-bandwidth-verification.md`。
