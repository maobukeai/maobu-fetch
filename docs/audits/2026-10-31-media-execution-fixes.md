# 媒体执行链整改与实际验证

日期：2026-10-31。对应 M08～M11、M16 的执行层，以及凭证检查补充审查。M06/M07/M13 另见 `2026-10-31-media-selection-fixes.md`；M16 容器算法的逐包校验与限制另见流式合并修复记录。本记录不是公网各平台完整下载验收，也不是发布验收。

## 逐项修改

| 编号 | 实际修改 | 行为证据 |
| --- | --- | --- |
| M08 代理与认证 | 新增 `media/network_policy.rs`，任务代理覆盖优先、空覆盖明确直连，全局手动/系统代理保持原策略；原生短链/抖音/Twitter/PikPak 和 yt-dlp 使用同一可取消通道。私网默认直连，明确任务代理仍生效。Cookie 使用原始作用域 jar；敏感自定义认证头若外部工具无法保证 origin 则中文拒绝，引导站点 Cookie 或原生 HTTP 直链。 | 实际 HTTP CONNECT、SOCKS5 认证、本地/远端 DNS、认证失败负向通过；实际 yt-dlp 下载合成站点 Cookie 到源且回环 Proxy-Authorization 未到源。HTTPS 上游使用系统证书校验；尚未跑受信任 HTTPS 上游的正向本地链。 |
| M09 共享限速 | 每任务短时随机认证的 `127.0.0.1` HTTP/CONNECT 代理，不解密目标 TLS。经HTTP(S)/CONNECT通道的请求/响应字节共用原生 `BandwidthScheduler`，任务所有连接共用一个 limiter，运行时限速/优先级原子更新；任务限速直接使用 bytes/s，不额外乘 1024。没有每个进程独占全局限额或固定分摊。 | 两媒体通道 + 原生 HTTP 在同一 64KiB/s 调度器的真实本地字节传输通过；动态任务限速、撤销限制、取消关闭全部通道通过。全产品 HTTP/BT/媒体混合最终验证由根回归记录负责。 |
| M10 真实统计 | 配置并发保留合法 1/2/4/8/16/32，显式 1 不改成 8。活动数来自当前已连接的 TCP 隧道/HTTP 流计数，速度来自真实已收字节，解析阶段为 0；不把片段上限当活动连接。结构化下载进度逐轨累加，未知总量保持未知。 | 真实 CONNECT 生命周期中活动计数为 1，取消/关闭归零；分轨进度、未知长度测试通过。统计是 TCP 隧道/流，无法进一步区分隧道内 HTTP/2 多路请求。 |
| M11 取消、超时与发布 | 媒体探测有 90 秒总上限，任务可用更短超时；Windows Job 在暂停状态启动进程后接管，取消/超时关闭整棵进程树并等待退出。stdout/stderr 和单行均有界，EOF 不替代进程终止。下载总期限使用统一有效重试策略，None保持无限，不暗加24小时；探测/本地转封装仍有独立操作上限。下载只写任务专属目录及 `.part`；结构化清单确定真实输出，验证后调用根原子发布，不再完成后重新联网命名。 | 实际 child + 已就绪 grandchild 在取消与总超时后停止写入；真实输出内存上限、重名 skip/rename 不改用户文件、外部路径拒绝、VoD 不误判直播通过。根负责最终 DB commit、启动恢复、原生 HTTP/HLS 的统一集成。 |
| M16 确定输入与流式合并 | `after_move` 清单给出实际轨道路径，必须是当前任务目录内非空普通文件。用 `hdlr` 区分音视频，调用流式 muxer，不扫描下载目录差集、猜名称/大小或整文件读入内存。无组件时显式禁止 yt-dlp 从 PATH 另找 FFmpeg，保留分轨走内置验证；不支持的容器明确要求 FFmpeg。 | 实际 yt-dlp 下载无编辑列表的 H264/AAC DASH → 内置合并 → FFprobe 两轨 → FFmpeg 完整解码通过；有 FFmpeg 的默认 DASH 也通过。普通包含 `edts` 的 DASH 仍要求 FFmpeg，不能声称所有 DASH 都能内置合并。 |

## 文件、隐私与交互补充

- 任务临时目录固定为 `.maobu-media-{任务ID哈希}.lumaget`，用 owner/来源与格式绑定防止不同任务或重试格式混用；随机合并候选名避免失败重试撞上已有暂存文件。无关用户文件不会因目录扫描被误选为输出。
- `PrivateConfig` 与 URL batch 文件先创建并设置当前用户 ACL 后写入；Cookie、代理密码、Referer、PO Token、签名 URL 不放命令行或日志。对子进程明确清空NO_PROXY/no_proxy并设置HTTP(S)/ALL代理环境，避免环境变量绕过应用代理/共享预算；宿主进程与全局环境不改。令牌仅本机短时存在；代理最多 64 个活动连接、头不超过 32KiB，取消后清掉全部连接。
- 认证仅依来源 origin/原始 Netscape domain、hostOnly、path、secure、expiry 规则发送。公开 Twitter Syndication 不接收 X 登录 Cookie，认证内容保留 scoped yt-dlp 回退。短链展开后也不会把 raw Cookie 重新绑定到另一 origin。
- 独立 UI 元数据检查属于控制流量，不伪造成下载任务或活动分片；任务自己的探测参与该任务和全局下载预算。任务配置/缓存使用该 Manager 的数据目录，隔离测试不会使用真实 AppData Store。
- 命名复用已获得的最小作者/日期/标题/编号元数据。UI 探测缓存 URL 与完整 Cookie 作用域只取哈希，私有缓存有 5 分钟 TTL 和 32 项限制，不保存 URL/认证头。根 Store 保存用户是否手编名称的意图；手编名优先保留，旧记录未知时保守保留，新自动名称缺信息可在开始前同预算探测一次，完成后不联网。
- 字幕从结构化清单读取，校验语言、已知字幕扩展名与目录范围，按真正完成文件名发布；尊重重名策略，不覆盖已有用户字幕。字幕在主文件journal之前完成，失败保留主暂存且不记录虚假完成；sidecar不覆盖主媒体journal。已经测得的rename名对应字幕名，已有用户文件保持不变。
- 明确直播停止才允许保存，实际 `is_live/live_status` 决定直播，任意 `.m3u8` 不是充分条件。FFmpeg 受控转封装并校验；没有工具时完整 fMP4 验证、TS 包结构/加密位/截断、FLV 标签/加密/截断验证后保留真实 `.ts/.flv/.mp4` 后缀。无工具的 TS/FLV 校验不等同于完整解码证明。
- M05 最小交互闭环补充：探测返回实际直播标记，external before_dl 通过 `record_media_live` 原子更新并记录本代可信观测；客户端提交 true、旧任务残留标记或单看 m3u8 地址都不能授权保存。普通暂停与明确停止保存的 UI/管理器完整行为由主回归另验。
- `cleanup_task_workspace` 只在根成功提交或用户删除后使用：校验固定任务目录和 owner，遇子目录、符号链接或非预期文件即保留；仅逐个移除已确认的任务普通文件，不递归清理用户目录。根接线与最终全产品回归另记。

## 凭证在线检查补充

`media_platforms/credential_check/` 替代原 600 多行重复客户端和假阳性逻辑。使用可注入、遵守应用代理且禁止重定向的 client，按每个官方 API 的目标 URL 选择 Cookie；多域 jar 不平铺。响应读取上限 1MiB、请求总时限 10 秒。网络不可达/响应中断返回“尚未判定”，与站点未确认登录分开。

抖音含 `ttwid/sessionid`、YouTube 含 `SID`、Twitter 返回普通 200 都不再直接标有效；必须有官方登录标志/身份响应。通用站点仅能确认连通，不能捏造登录成功。重定向不携凭证继续请求。根负责 `lib.rs` 的原始 jar 与新 client 接线。

## 已执行证据

| 验证 | 实际结果 |
| --- | --- |
| `cargo check --lib`（首次执行层） | 通过；日志 `%TEMP%/maobu-media-execution-check.log`。之后改动仍需最终统一 Cargo。 |
| 当前已构建测试 EXE 的 `media::` 过滤 | **67 通过，0 失败，1 个内部 child fixture 显式忽略**，9.10s；包含生产格式 helper、DRM、缓存/命名、字幕、TS、真实进程树、通道与共享预算。日志 `%TEMP%/maobu-media-execution-tests.log`。 |
| 当前 EXE 凭证检查 / Cookie scope | 3/3、2/2 通过，含真实本地请求、禁止重定向、网络不可达区别，以及域/path/secure/expiry/同名排序。实际Windows junction/reparse拒绝与工作目录清理亦已通过。 |
| 真实媒体工具隔离 E2E | `MAOBU_TEST_EXTERNAL_MEDIA=1 MAOBU_TEST_EXTERNAL_MEDIA_ONLY=1`：**1通过、0忽略，16.69s，9个真实场景**。progressive 69169字节与源SHA相同；FFmpeg DASH 69169字节；无edts内置DASH 69227字节；TS无工具`.ts`/有工具`.mp4`；实际task probe的Cookie/私有缓存；edts拒绝保留2轨；实际取消不发布；真实yt-dlp经2次认证upstream CONNECT，69169输出字节exact source，16KiB/s任务预算实际耗时8204ms。所有可播放场景FFprobe双轨与FFmpeg完整解码通过。日志 `%TEMP%/maobu-media-tools-only.log`。 |
| 公网匿名抖音解析 | 移除固定第三方匿名 Cookie 后，已有 EXE 显式 `test_live_douyin_aweme_detail_fetch` **1/1 通过**，3.18s，实际描述、格式与 HTTPS 直链断言成功；不是下载验收。默认源码已明确 ignore，后续显式运行需 `--ignored`。日志 `%TEMP%/maobu-douyin-live-probe.log`。 |
| 扩展 / TS 选择器 | 最后重新执行 **193/193**、**17/17** 通过。 |
| 用户授权 B 站完整样本 | 匿名实际生产 probe/download，57:46、360P AV1/AAC、143358482字节，源轨/输出SHA与全片解码实际通过，64.87秒；详见 `2026-10-31-bilibili-user-sample.md`。 |

真实工具首轮发现缺失 `%(field)j` 输出非 JSON 的 `NA`，已采用 `%(field|null)j` 明确缺省值修复；保留严格 JSON 校验，没有吞掉解析错误。默认 edts DASH 的拒绝也未放宽，而是分别测试 FFmpeg 路径和真正符合内置支持范围的 fMP4。

新增的未知音轨、真实任务probe/取消、edts保轨负向、Windows junction、字幕先于主journal等回归均已实际通过。真实Chrome/Edge到Rust桥及离线浏览器回退另见 `2026-10-31-bridge-browser-validation.md`。用户新提供B站真实样本已另外完成显式opt-in完整下载，不将合成本地9组扩大为该公网结论。最后新增的大小估计与直播标记 helper 由最终统一 Cargo 验证，本文不提前增加 Rust 通过计数。

## 依赖、兼容与未验证范围

直接声明 `native-tls 0.2.18` / `tokio-native-tls 0.3.1`，用于 HTTPS 上游代理以 Windows 系统信任链建立 TLS；它们原已在锁文件的传递依赖图中，未另引入 TLS 实现。使用 MIT/Apache-2.0 许可证的现有依赖系；新增 Windows Job/ToolHelp 仅为既有 windows-sys 特性。没有安装/下载/打包媒体工具，测试只使用本机已存在工具。

端口、`/v1` 版本/HMAC/头、Tauri 命令名保持兼容。能力/格式响应新增默认字段见选择器记录；请求只在已有 media 对象追加可选 is_live，缺失默认 false，旧服务端忽略未知字段，服务端仍以本代真实观测授权停止保存。没有更改用户已有完成文件或真实账号凭证。

尚不能声称通过：真实账号、会员与地区限制、DRM（明确拒绝）、全部平台全流程下载、长期直播、真实私有 HTTPS 代理正向信任链、非HTTP媒体传输协议（如RTMP）、所有异常容器类型、大体积多 GB 压测、完整安装包/升级/商店验证。B 站公开多 P 探测被 HTTP 412 限制仍如实保留。各平台状态须按具体日期、工具版本、测试层级和证据登记。
