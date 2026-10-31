# Maobu Fetch 的受控 librqbit 补丁

版本仍为官方 `librqbit 9.0.0`。上游仓库：[ikatson/rqbit](https://github.com/ikatson/rqbit)，crate VCS commit：`de2b107ea9a1dd466ce4c4f7fd31929d7b481f36`。

本目录从本机 Cargo 官方 registry 缓存的 `.crate` 提取；提取前实测 SHA-256 与原 Cargo.lock 完全相等：

```
10003b9b157ef1d8352b4ff2e2025fa1544453d40539741779ee918028aba75b
```

保留生产 `src/`、原始归一化与原始 Cargo.toml、README、build.rs。没有复制上游下载样本、种子夹具、示例、前端 webui 或独立 Cargo.lock；项目只启用已有的 `rust-tls` 特性。`LICENSE` 从同一 commit 的上游根目录获取，保留 Igor Katson 的版权与 Apache-2.0 授权。没有改本机 registry 源码。

## 为什么需要补丁

- 上游每秒额度同时是 governor 的 burst 容量。16 KiB 协议块遇到 1–8 KiB/s 等低速配置时，`until_n_ready()` 直接返回 InsufficientCapacity，不能正常限速。
- torrent 级 limiter 仅在 AddTorrentOptions 中可配置，Live 私有成员不能动态更新。每次更改限速都删除/重新添加会导致重新检查文件；每任务拆会话又会增加 DHT 与后台资源。本补丁保留一个 Session、一个 DHT，将真实 limiter 在各状态间共享并公开只读引用。
- 上游会话恢复会自动启动上次未暂停的 torrent，绕过应用的排队、默认不做种和显式恢复政策。
- pause/unpause/文件选择的持久化更新失败原来只打印警告，应用无法知道恢复数据是否保存成功。
- 上游暂停没有同步刷新 bitfield，`DiskBackedBitV::flush(false)` 也仅排队；正常退出不能据此宣称恢复快照已落盘。
- fastresume 的抽样校验用 `check_piece().is_err()` 判断，漏掉哈希不匹配的 `Ok(false)`。
- 在初始校验过程中恢复，原生任务可以丢掉新 peer stream 并停在 Paused；本地真实 seeder 回归发现此问题。

## 修改范围

只修改以下八个生产文件，其他上游生产源码保持字节一致：

1. `src/limits.rs`：把大块按真实 quota 分次预留，额度与 limiter 一起原子替换；重设相同值不补发 burst；补 `Default`。
2. `src/torrent_state/mod.rs`：torrent options 使用同一个 `Arc<Limits>`，公开 `ManagedTorrent::ratelimits()`，状态切换不丢配置；初始校验期间保留最新恢复意图和 peer stream；幂等暂停；提供暂停态同步刷新和只读重检入口；提供真实选片用的动态 streaming_priority 开关。
3. `src/torrent_state/live/mod.rs`：复用上述 limiter；启用 streaming_priority 时将全部已选文件的首尾分片加入原生优先队列，保留播放器 stream 的更高优先级，未选文件不会进入队列。
4. `src/session.rs`：构建共享 limiter；新增默认 false 的 `restore_paused` / `restore_recheck`（仅应用启用）；暂停真正刷新 bitfield，持久化更新/删除失败沿 Result 返回，失败保留可重试的原任务；提供不构造会话、不联网、不打开下载内容的离线持久记录清理；文件和远端 magnet 元数据统一检查零除数、总长溢出、分片索引容量和哈希数量，避免上游 Lengths 构造触发 panic。
5. `src/bitv.rs`：同步 flush 执行真实写入和 `sync_all`；后台异步快照与同步快照共享 revision，旧队列不能覆盖新快照；磁盘错误保留并使后续同步刷新失败。
6. `src/session_persistence/json.rs`：JSON、种子元数据、首次 bitfield 写入临时文件并 `sync_all` 后替换；元数据写入错误不吞；JSON提交失败恢复内存条目，删除可重试。
7. `src/torrent_state/initializing.rs`：单行修正抽样哈希判断，只有 `Ok(true)` 才算通过。
8. `src/create_torrent_file.rs`：单行修正整分片边界的创建逻辑，长度恰好整除 piece_length 时不额外追加空分片 SHA-1。严格元数据校验准确暴露了该问题，不放松哈希数量校验来迁就无效种子。

这是同版本、无新增第三方依赖的源码补丁，不是升级到未经验证的库版本。根 Cargo.lock 仅去掉该 package 的 registry source/checksum，以 `[patch.crates-io]` 指向本目录；其他锁定版本保持原值。

## 回归与复查

`scripts/audit/verify_librqbit_vendor.py` 验证原始 archive 的 SHA 与非补丁生产文件一致性。项目 `bt::regression_tests`、`bt::swarm_tests`、`bt::crash_tests` 使用真实原生会话、本地 tracker/seeder 和实际被终止的子进程，覆盖动态 limiter、选择、校验、持久化失败、恢复与隔离。测试结果以修复报告和最终 Cargo 输出为准，不把源码审查当成行为测试通过。

更新上游版本时应先核实这些修复是否已经合入，重新运行回归后再去掉对应补丁。不要只更新 crate 版本号或将补丁复制到机器私有的 Cargo registry。
