# 官方 Ubuntu 公网元数据与受限协议探针

执行时间：2026-10-31 05:29 UTC。此结果验证当前机器直连网络的官方 Tracker/公网 peer 协议可达性，**不代表生产 librqbit 引擎已完成公网下载验收**。

## 来源与边界

先用网页工具核对[Ubuntu 官方发布目录](https://releases.ubuntu.com/24.04/)，选择其中的[Ubuntu 24.04.5 Server torrent 元数据](https://releases.ubuntu.com/24.04/ubuntu-24.04.5-live-server-amd64.iso.torrent)。通过官方 HTTPS 实际获取 311,682 字节，元数据 SHA-256：

```
81d2234a304fbed595c78df9ec243ae7695263f591f3dabedcb808e5cd7615c8
```

元数据仅保存在独立系统临时目录，没有写入用户下载目录或数据库。没有启动正式用户客户端，没有下载 ISO 文件，没有向 peer 上传数据。

`scripts/audit/ubuntu_bt_probe.py` 只允许启动工作区 Cargo 测试 EXE 的纯元数据 helper；该 helper 显式调用真实 `bt::inspect_torrent_bytes`，校验临时路径、1MiB 上限与上述 SHA-256。后续 Python 手动协议探针限制官方 Ubuntu Tracker、至多 3 个公网 IPv4 peer、45 秒硬期限、累计请求和读取分片 payload 至多 2MiB。按 16KiB 顺序请求，流式计算 SHA-1，不写入分片文件，不宣称验证超出预算的整片。

## 实际结果

| 项目 | 结果 |
| --- | --- |
| 生产元数据解析函数 | 通过，infohash `a3b4136a174e30b121a6bb6ab75b577ea7eda5f3` |
| 原始 ISO 声明长度 | 4,080,486,400 字节；没有下载该完整文件 |
| 分片大小 | 262,144 字节，小于 2MiB 上限 |
| 官方 HTTPS Tracker | `https://torrent.ubuntu.com/announce` 返回 1 个公网 IPv4 peer，约 1,062ms |
| 公网握手 | 1 个 peer，握手返回的 infohash 匹配 |
| 请求/读取 payload | 262,144 / 262,144 字节，仅一个分片 |
| 总协议读取 | 262,434 字节，包含握手和控制消息 |
| 校验分片 | 索引 13,831，SHA-1 `25f06e3c864547e66e276ed3a7f775e9bd15ab3e` 与官方元数据匹配 |
| 退出 | 已发送 stopped announce，全部 socket 关闭 |
| 本轮协议探针时长 | 20,187ms，含生产解析 helper；此前元数据 HTTPS 获取另行约 5.85 秒 |

原始结构化结果：`2026-10-31-bt-public-probe.json`。结果不保存公网 peer IP，也不包含 Cookie、用户令牌或私有种子 URL。

## 不能外推的结论

- 不是生产引擎公网传输路径测试。公网数据部分是独立、严格有界的协议脚本；生产引擎下载、恢复、限速与上传仍由受控本地真实 Session/manager 测试验证。
- 没有验证完整 ISO 的 SHA-256、所有分片、长时间做种、NAT 入站、IPv6 或各运营商互通。
- 单个官方样本当时可达，不能保证未来相同 Tracker/peer 持续在线，也不代表任意私人磁力链接可用。
