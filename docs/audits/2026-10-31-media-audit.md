# 猫步下载器媒体与站点适配深度检查

检查日期：2026-10-31（Asia/Shanghai）；代码基线：`7ecb065`，项目版本 `0.10.11`。

本报告主体为初次审查基线和修改建议；后续整改与实际下载验收另有记录，B站行已更新用户授权样本结果。初次审查时媒体产品代码未修改。不能把“支持域名”“提取器存在”“单元测试通过”或“解析出格式列表”解释为完整下载通过。

## 1. 结论

发现 6 项 P1、10 项 P2。最紧急的是原生 M3U8 下载器：隔离测试实际复现错误 Range、未知加密、直播快照、损坏旧切片及合并失败仍返回成功等 5 条路径。另有默认媒体格式可能选择无声视频、B 站选集前后端没有接通的问题。

逐站在线测试覆盖了抖音、B 站、YouTube、TikTok、Twitter/X、微博、小红书、西瓜、快手及 5 个网盘。YouTube 和微博的公开样本成功返回元数据；PikPak、百度、蓝奏成功列出分享条目；B 站在本环境遭遇 412/SSL 错误；部分站点要求新 Cookie。快手没有当前版本专用提取器，本次公开页面样本返回 Unsupported URL。其余结果见矩阵，不能外推到整个站点。

初次审查没有读取浏览器 Cookie、用户数据库或个人下载，没有安装或更新媒体组件，也没有完整下载公网视频；后续用户授权的 B 站样本已另行完成全片下载与解码，见矩阵及独立验收报告。初次 M3U8 行为验证只使用本机 `127.0.0.1`、数个字节的合成响应和独立临时目录。真实账号、付费清晰度、真实直播长时间录制、网盘大文件下载仍需对应验收。

## 2. 验证方法和证据

- 系统已有 pipx 安装的 `yt-dlp 2026.07.04`，通过已安装 Python 环境运行；没有将其当作 GitHub 独立 EXE 做哈希比较，因为它们是不同分发形式。
- 在线测试使用 `--dump-single-json --no-playlist --no-warnings --no-check-formats --skip-download`，并加 `--ignore-config`、超时和零重试，防止使用本机配置中的凭证或产生无界等待。第二轮使用产品代码默认 Chrome/131 UA；YouTube 使用产品无 Cookie 分支的 extractor args。
- `--no-check-formats` 不验证 CDN 地址可下载性；本报告将其标为“元数据通过”，不是下载验收。
- 第一轮 2026-10-31 10:08:53～10:09:00；第二轮 10:09:43～10:09:46；额外默认格式证据 10:15:50。精确 UTC 时间、样本和结果保存在 `2026-10-31-media-results.json`。
- 根检查任务首轮运行完整 Rust 测试：1,212 通过、9 忽略，独立 Range 集成测试 9 通过；新增完成提醒兼容测试后最终为 1,213 通过、9 忽略，Range 仍为 9 通过。其中 `test_live_douyin_aweme_detail_fetch` 通过。已核对该测试有 `res.is_ok()`、非空标题、非空格式和 HTTPS URL 断言；它确实验证本项目原生抖音解析，未验证媒体文件下载。
- `scripts/audit/media_probe.py` 是可复用、无 Cookie、只解析元数据的脚本。其 `--help` 已验证，语法通过；记录数据来自同等参数的两轮审查命令，不冒称该保存脚本已经完整执行。
- `scripts/audit/run_m3u8_probe.ps1` 已实际运行。它直接导入未修改的 `downloader.rs`、`parser.rs`、`crypto.rs`，仅将任务存储、事件、限速边界替换为无副作用替身。它验证真实 HTTP/切片/合并实现，不覆盖 Tauri 窗口、完整 DownloadManager 和数据库状态机。输出为观察结果，不是宣称这些错误路径测试通过。
- `scripts/audit/run_cloud_share_probe.ps1` 已实际运行，直接导入 5 个未修改的网盘模块，单站 25 秒总时限、并行、无 Cookie，只调用 `inspect_*_share`。未调用直链解析、转存、保存、下载，不连接用户数据库。时间为 10:19:54～10:19:59；不重复记录夹具中的分享提取码。

## 3. 各平台适配矩阵

| 平台 | 本项目实现路径 | 本次在线结果 | 尚未验收的范围 |
|---|---|---|---|
| 抖音 | `media.rs:211` 优先原生 SSR/详情 API，失败回退 yt-dlp；直播另走页面/API | 项目原生联网测试样本 `7694589931460594953` 通过；同样本 yt-dlp 无 Cookie 回退要求 Fresh cookies | 短链、图集全图、私人内容、真实清晰度、完整文件、实时直播持续录制及停止保存 |
| B 站 | 整改后窄正规化语言路径；实际选集接线；默认严格有声格式与受控合并 | 用户授权 `BV1VeHQ6tEaS` 匿名完整57:46、360P AV1+AAC、143358482字节、全片解码与源/输出SHA实测通过（yt-dlp2026.07.04、FFmpeg9.0）；详见 `2026-10-31-bilibili-user-sample.md`。初次其他样本412/SSL记录仍保留 | 不外推全站；登录/会员/地区清晰度、1080P/4K、番剧、多P、字幕及直播仍需各自验收。选集代码缺陷已整改但公网多P受限未算通过 |
| YouTube | 通用 yt-dlp，加按凭证分支构造的 player_client/PO Token 参数 | 公开样本 `YE7VzlLtp-4` 成功，27 个格式；旧样本 `BaW_jenozKc` unavailable，不能据此判全站失败 | 实际 CDN 下载、1080P/4K 有声合并、Shorts、直播回放、字幕、登录和年龄限制；默认无声格式问题见 M06 |
| TikTok | 已识别平台，通用 yt-dlp | 公开历史样本返回 Unexpected response from webpage request | 地区限制、登录、图集和完整下载；本结果不能区分站点变化、样本变化和环境风控 |
| Twitter/X | 优先 Syndication 原生 API，失败回退 yt-dlp；Spaces 走 yt-dlp | 原生同等 HTTP 请求对星战样本 200、有核心字段但无媒体格式；yt-dlp 对星战样本在 amplify 页面报 500 Domain Not Found；旧样本无视频 | 原生 JSON 转换后真实格式、普通视频/动图/多图、登录内容、Spaces 完整下载。未把回退失败判成整个原生入口失败 |
| 微博 | 平台识别，通用 yt-dlp；图集有 URL 类型识别 | `4189191225395228` 成功，2 个格式，有标题 | 完整视频、全图集、登录内容、过期直链、字幕 |
| 小红书 | `detect_platform` 为 Unknown，依赖 yt-dlp XiaoHongShu 提取器 | 历史公开样本返回 No video formats found | 图文笔记、带签名分享、登录态、真实视频下载；没有本项目专用站点适配 |
| 西瓜视频 | Unknown，依赖 yt-dlp Ixigua 提取器 | 历史公开样本要求 Cookies（不一定需要登录） | 新 Cookie、清晰度、合并和完整下载；没有本项目专用站点适配 |
| 快手 | Unknown，通用提取器；已安装 yt-dlp 未列出 Kuaishou/Kwai 专用提取器 | 此次短视频页面样本 Unsupported URL | 需要专门适配或经验证的浏览器媒体探测流程；不能宣称支持粘贴网页后解析下载 |
| PikPak | 原生分享列表、密码、直链刷新；前端有多选入口 | 直接运行原生模块，成功列出 1 个文件；没有解析/下载直链 | 分享密码、分页完整性、目录超过上限、直链过期刷新、真实下载与哈希 |
| 夸克 | 原生分享 token、目录遍历、转存/直链 | 原生模块返回 Ok，但 0 条目/0 文件；本次不能认定列表通过，也不能据此认定全站故障 | 样本有效性、登录态、空列表与错误区分、转存是否清楚告知、超过 300 文件/40 目录、真实下载 |
| 百度网盘 | 原生分享页面/API、密码、目录树、Cookie/转存/直链 | 原生模块无 Cookie，使用既有夹具提取码，成功列出 2 条目（1 文件）；未解析直链或转存 | 授权账号、目录分页、转存行为、速率和完整性 |
| 蓝奏云 | 原生网页/iframe/API、密码及目录入口 | 直接运行原生模块，成功列出 49 条目；未解析/下载直链 | 密码/目录、页面挑战变化、完整下载 |
| 123 云盘 | 原生分享 API、提取码、目录递归、直链 | 原生模块返回 requires_password=true、0 条目；只验证了该样本密码状态反馈 | 需有效公开或有授权提取码样本、多于 100 条目、受限分享、目录路径、完整下载 |

公开样本链接（全部为测试输入，未记录任何签名 CDN 地址）：

| 平台 | 输入 |
|---|---|
| 抖音 | https://www.douyin.com/video/7694589931460594953；历史回退样本 https://www.douyin.com/video/6961737553342991651 |
| B 站 | https://www.bilibili.com/video/BV13x41117TL；https://www.bilibili.com/video/BV1bK411W797；后续用户授权完整样本 https://www.bilibili.com/video/BV1VeHQ6tEaS/ |
| YouTube | https://www.youtube.com/watch?v=YE7VzlLtp-4；旧样本 https://www.youtube.com/watch?v=BaW_jenozKc |
| TikTok | https://www.tiktok.com/@leenabhushan/video/6748451240264420610 |
| Twitter/X | https://twitter.com/starwars/status/665052190608723968；旧样本 https://twitter.com/i/status/560070183650213889 |
| 微博 | https://m.weibo.cn/status/4189191225395228 |
| 小红书 | https://www.xiaohongshu.com/explore/6411cf99000000001300b6d9 |
| 西瓜 | https://www.ixigua.com/6996881461559165471 |
| 快手 | https://www.kuaishou.com/short-video/3x5kdtj4t9fqws6（仅形态测试输入，未核实作品仍存在） |
| PikPak | https://mypikpak.com/s/VOveL7ZI01ViAz9VVKGgSWDlo2 |
| 夸克 | https://pan.quark.cn/s/69ba75a686aa#/list/share |
| 百度 | https://pan.baidu.com/s/1uqSBOwEkkw-NdAenNndoqA（分享提取码沿用仓库既有夹具，不重复公开） |
| 蓝奏 | https://www.lanzoui.com/u/yoyodadada |
| 123 | https://1683912.share.123pan.cn/123pan/z3h9-rtFzh（原输入含 notoken=1 查询参数） |

除快手形态样本和项目自身抖音测试样本外，主要输入来自已安装/官方 yt-dlp 提取器回归样本。历史样本可失效，结果需同时记录链接和日期。

## 4. 实际复现的 M3U8 问题

### M01 · P1：合并复制失败被吞掉，正确切片被清理

位置：`src-tauri/src/m3u8/downloader.rs:477`、`:484`；后续调用见 `src-tauri/src/manager.rs:3237`。

触发：最终临时目标是只读旧文件，正确切片已完成下载。`rename` 失败后调用 `copy`，不检查返回值，随后删除合并文件和所有正确切片，并返回 `Ok(task)`。

隔离结果：源内容 `CORRECT`（7 字节），已有目标 `STALE`（5 字节）；返回 `result_ok=true`，报告 7 字节，磁盘仍是旧的 5 字节，切片已经不存在。没有把完整 manager/UI 的完成事件称作实机复现，但下载器成功结果与磁盘事实已确定矛盾。

建议：复制失败必须返回错误，保留合并文件和分片；成功后校验实际目标长度，再清理。manager 必须要求存在且为完整普通文件，禁止“临时路径不存在也完成”。验收应注入权限不足、磁盘满、rename/copy 失败并检查恢复数据保留。

### M02 · P1：BYTERANGE 收到 200 全量响应仍拼接，产生重复损坏文件

位置：`src-tauri/src/m3u8/downloader.rs:246`、`:253`。

触发：播放列表将同一 8 字节对象分为 `4@0`、`4@4`；服务器忽略 Range，均返回 200 和完整对象。代码对所有 2xx 接受，没有校验 206、Content-Range 和预期范围长度。

实际结果：期望 `AAAABBBB`（8 字节）；返回成功并输出 `AAAABBBBAAAABBBB`（16 字节）。

建议：字节范围请求验证状态、范围和最终长度；服务器忽略 Range 时明确拒绝，或只在完整对象长度/切片边界有可靠依据时做受控切取。补 200、错误 206、416 和截断响应测试。

### M03 · P1：SAMPLE-AES / 未支持加密未拒绝，密文也返回成功

位置：`src-tauri/src/m3u8/parser.rs:179`；`src-tauri/src/m3u8/downloader.rs:201`、`:223`、`:330`。

触发：声明 SAMPLE-AES 的合成列表。解析器接受该方法，下载器只处理 AES-128，其余分支直接跳过解密并写入字节。

实际结果：输出原始 `ENCRYPTED` 9 字节，返回 `Ok`。此测试没有尝试解密或绕过真实 DRM。

建议：对未支持加密和 DRM 类型在请求媒体片段之前给出明确拒绝；不要把加密媒体当成功文件。对 METHOD、KEYFORMAT、非法/缺失 key 和 IV 增加拒绝测试。

### M04 · P1：切片只按“存在且非空”信任，损坏内容/旧资源可直接合并

位置：`src-tauri/src/m3u8/downloader.rs:99`、`:192`；manager 在 `:3199` 分流到 M3U8，早于 HTTP 分支的远端 ETag/Last-Modified 检查 `:3270`。

触发：临时分片 `.seg0` 已有 3 字节错误内容 `BAD`；真实服务器应提供 `CORRECT`。代码不校验长度、校验和、片段 URL/序列号或远端验证器，直接跳过该片段。

实际结果：返回成功，输出 `BAD`（3 字节），未恢复正确源。重启后列表变化或分片损坏时存在相同风险。

建议：落盘分片清单绑定规范化播放列表、media sequence、URL、范围、实际长度和完整性信息；断点恢复时核验，不匹配时明确重下该片段。现有 HTTP 下载的 Range 校验不能覆盖此独立分流。

### M05 · P1：直播 M3U8 仅下载首个窗口就结束

位置：`src-tauri/src/m3u8/downloader.rs:39`、`:65`、`:400`；`parser.rs:57` 的 `is_endlist` 在下载器中未使用。

触发：提供无 ENDLIST 的直播列表，只有当前一个片段。下载器解析一次，然后按有限任务下载合并，没有轮询新列表、按 sequence 去重或等待用户结束。

实际结果：`FIRST_WINDOW` 下载后立即返回成功。原生入口不能据此宣称持续直播录制正常。带 MediaSelection 的部分直播会走 yt-dlp，是不同路径，不能用其表现替代原生验证。

建议：先明确直播分流：实现有界刷新、序列去重、取消与保存语义，或在该原生入口明确拒绝直播并引导现有已验证的录制入口。

观察输出原文（仅合成内容）：

```text
range_ignored: result_ok=true reported_bytes=Some(16) actual_bytes=16 content="AAAABBBBAAAABBBB" shard_exists=false
unsupported_encryption: result_ok=true reported_bytes=Some(9) actual_bytes=9 content="ENCRYPTED" shard_exists=false
live_snapshot: result_ok=true reported_bytes=Some(12) actual_bytes=12 content="FIRST_WINDOW" shard_exists=false
corrupt_existing_shard: result_ok=true reported_bytes=Some(3) actual_bytes=3 content="BAD" shard_exists=false
merge_copy_failure: result_ok=true reported_bytes=Some(7) actual_bytes=5 content="STALE" shard_exists=false
```

## 5. 其他功能与控制问题

### M06 · P1：默认清晰度选择可以选中无声视频

位置：`src-tauri/src/media.rs:1416`；`src/components/modals/NewTaskDialog.tsx:988`；`src-tauri/src/manager.rs:2470`。

`parse_single_format` 对单格式统一设置 `requires_ffmpeg=false`，包括 `has_audio=false` 的 DASH 视频。前后端默认都从 `has_video && !requires_ffmpeg` 候选中选最高高度，且该候选优先于“合并音视频”。即使 FFmpeg 已安装，仍可能默认选无声视频。

实际元数据：YouTube 公开样本的最高视频候选为 `248-sr`、1080P、VP9、`acodec=none`，同时存在 1 个音视频合一候选。触发条件由真实元数据确认；默认选中行为根据上述确定代码路径推导，未操控真实桌面下载完整视频。

建议：默认选含声音的可用格式；若用户选择视频单轨，清楚标注“仅视频，无声音”。有 FFmpeg 时默认合并最高画质，无 FFmpeg 时选择最高有声单文件。以视频单轨高度高于有声候选的数据做回归。

### M07 · P2：B 站多 P / 番剧选集只有前端，没有后端数据

位置：`src-tauri/src/media.rs:607`、`:578`；`src-tauri/src/models.rs:1336`；`src/components/modals/NewTaskDialog.tsx:1078`；`src/components/EpisodePicker.tsx`。

所有媒体 probe 返回的 `episodes` 都为空，Rust 仓库没有构造 `MediaType::Collection` 的分支；yt-dlp 始终 `--no-playlist`。前端的选集组件和 collection 分支因此收不到数据。内置兼容表却把 B 站“多P合集、番剧剧集”列为正常下载。

建议：实现独立、有上限且可取消的合集/分 P 元数据探测，逐条提供真实入口和权限反馈；否则收窄产品说明。验收区分单视频、分 P、公开合集、需要权限的番剧，不得绕过权限或 DRM。

### M08 · P2：媒体解析与 yt-dlp 下载没有接入应用代理

位置：`src-tauri/src/media.rs:590`、`:655`；`media_platforms.rs:354`、`:493`、`:1329`、`:1460`、`:1841`；网盘模块各自的 Client builder。

这些路径没有消费应用的全局代理/任务覆盖配置，yt-dlp 命令也未传 `--proxy`。可能受系统/环境代理影响，但不等于遵守应用设置。用户设置代理或直连后，不同下载类型可能走不同网络。

建议：统一代理解析和凭证保护；元数据、短链、直链刷新、密钥/切片及下载都采用同一任务策略。用本地可观测代理分别验证“直连/全局代理/任务代理”。

### M09 · P2：yt-dlp 路径的“全局限速”按任务重复套用

位置：`src-tauri/src/media.rs:712`；共享调度器见 `src-tauri/src/manager.rs:124`、`:208`。

每个 yt-dlp 子进程拿到完整全局限额；任务限额大于零时又直接优先替代全局限额。该函数不经过共享 BandwidthScheduler，也没有在运行中同步限额更改。因此多个此类任务可能合计超过全局上限，且任务设置可绕过更低的全局值。

建议：实现跨媒体子进程与原生下载共享的预算，保留单任务上限，两者同时生效；添加混合 HTTP+媒体、多媒体、动态限速测试。本项为确定代码路径审查，未用大流量公网压测。

### M10 · P2：媒体活动连接数展示来自参数而非真实连接

位置：`src-tauri/src/media.rs:724`、`:734`；`m3u8/downloader.rs:85`、`:158`。

yt-dlp 的 concurrent-fragments 参数不等于实际 HTTP 活动连接，解析/单文件协议/只有少量切片时也被设置为完整连接数。任务显式 1 连接在媒体函数还会回退 `settings.connections_per_download.max(8)`。原生 M3U8 的连接数也按 worker 配置反复发出，队列不足时仍不减。

建议：区分“配置并发上限”与“当前活动连接”；无法观测的外部进程显示未知，不伪造。保留任务显式 1 的设置，不用连接数字段表示“已开始解析”。

### M11 · P2：yt-dlp 直接写最终文件，探测/命名阶段缺乏超时和取消

位置：`src-tauri/src/media.rs:969` 的 `--no-part`；`:635`、`:1189` 的 `command.output().await`。

部分媒体任务直接写最终文件名，半成品会提前出现在目标位置。探测子进程及下载后的再次命名探测没有产品级总超时/取消 token；慢站点可让界面长时间等待，媒体已下载完仍因再次联网迟迟不报告完成。任务下载循环有取消，不代表这两个 output 调用可取消。

建议：普通点播使用任务隔离临时输出，确认后原子替换；直播另明确保存策略。将 probe 设计为可取消、有总时限的操作，复用已有元数据进行命名，避免完成后再次探测。此项未做长时间挂起实机测试。

## 6. 网盘完整性、状态与可维护性

### M12 · P2：目录列表被静默截断

位置：`baidupan.rs:344`、`pan123.rs:121`；`quark.rs:204`、`:205`、`:335`；`pikpak.rs:264`、`:265`、`:448`。

百度、123 每个目录固定取第一页 100 条，没有后续分页循环；PikPak 限 200 文件/35 目录，夸克限 300 文件/40 目录，达到上限返回当前列表但缺少“结果未完整”标识。大目录里用户执行“全选”，实际可能只下载部分文件。

建议：实现分页、去重、可取消和明确截断反馈；如保留保护上限，返回 `truncated` 及继续加载入口。用可控 mock 服务验证 101/201/301 个文件、深层目录、分页失败，不依赖真实网盘大包。

### M13 · P2：PikPak 媒体探测伪造分辨率和音视频属性

位置：`src-tauri/src/media.rs:383`、`:397`、`:425`。

未探测文件流信息即标“原画1080P/4K”，固定 `1920×1080`、有视频/有音频，且存在固定 `.mp4` 回退。分享实际可能是图片、压缩包、低分辨率或纯音频，不能把站点/文件名当作质量证据。

建议：未知尺寸显示待获取；原文件下载只展示真实文件元信息，需要媒体属性时再做有界探测，不生成虚假 1080P/4K 标签。

### M14 · P2：内置兼容等级缺少可审计的测试时间

位置：`src-tauri/src/store.rs:1863`、`:1878`。

多站点直接 seed 为 `verified`，`known_issues=[]`，`last_tested_at=''`；无探测任务更新这些数据。它是静态说明表，不能作为“当前实时兼容”证据，本次 B 站联网失败和选集缺失也不能由该标志反证。

建议：等级绑定版本、样本、验证层级和日期；明确“元数据/下载/合并/登录”各自结果。未跑项目应显示未验证，历史已验证要显示日期；不增加后台联网扫描。

### M15 · P2：联网测试可在失败时显示通过，降低报告可信度

位置：`pikpak.rs:689`、`:725`；`quark.rs:834`；`baidupan.rs:997`；蓝奏真实测试 `lanzou.rs:628` 默认忽略。

PikPak 公开分享测试打印结果但没有断言；保护分享网络失败直接 return。夸克、百度失败时打印“跳过在线断言”后正常返回。离线、接口变更、过期样本可能呈现 test ok。抖音实际联网测试不属于这类，其成功断言已核实。

建议：联网测试单独 opt-in，明确失败/跳过原因；固定逻辑用本地 fixtures/mocks 保证断言。对下载直链输出做脱敏，避免 Debug 打印完整临时授权链接。报告不能用这些 test ok 作为网盘验收通过。

### M16 · P2：内置音视频合并依赖整个目录新增文件猜测，并整文件载入内存

位置：`src-tauri/src/media.rs:1008`、`:1018`、`:1034`；`src-tauri/src/media_muxer.rs:369`、`:372`、`:436`。

合并器没有使用任务独立目录或 yt-dlp 明确输出清单，而按目录前后差集猜本任务的两个文件，使用名称/大小猜音视频。并发任务、用户新放文件或字幕都可能干扰；单新增文件甚至直接重命名。真正 fMP4 合并又整文件读入视频和音频，并建立额外输出 Vec，大文件可能占用数倍媒体大小的内存。

建议：每任务独立工作目录，通过结构化输出拿到确定轨道路径；用容器轨道信息区分而不是文件大小；流式复制媒体数据。当前前端还会要求安装 FFmpeg，部分内置回退路径未被正常 UI 开放，因此本项应按入口分别验证，不能假设所有用户都会进入。

## 7. 媒体工具固定版本和可信清单核实

本轮只请求官方 API、HEAD 和校验文本，没有下载安装程序。核实时间 2026-10-31，以下与 `src-tauri/src/media_tools.rs:22`～`:33` 常量一致：

| 组件 | 固定版本 | 远端 HEAD | 字节数 | 官方 SHA-256 |
|---|---|---|---:|---|
| yt-dlp 独立 EXE | 2026.07.04 | 200 | 18,226,085 | `52fe3c26dcf71fbdc85b528589020bb0b8e383155cfa81b64dd447bbe35e24b8` |
| FFmpeg essentials ZIP | 8.1.2 | 200 | 109,728,040 | `db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec` |

依据：

- [yt-dlp 官方 Release API](https://api.github.com/repos/yt-dlp/yt-dlp/releases/tags/2026.07.04)：资产 `yt-dlp.exe` 的 size/digest 与代码一致；[固定下载地址](https://github.com/yt-dlp/yt-dlp/releases/download/2026.07.04/yt-dlp.exe) HEAD 成功。
- [Gyan 固定 FFmpeg 包](https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip) HEAD 成功；[官方校验文本](https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip.sha256) 与代码一致。
- `media_tools.rs:1263` 采用 ZIP `enclosed_name` 并只提取 ffmpeg/ffprobe，是已存在的有效防护。本轮没有执行安装、取消、重试或包内清单验收。
- [yt-dlp 官方支持清单](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md) 自身说明列出提取器不保证当前可用；[PO Token 官方说明](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide) 用于核对 YouTube 存在客户端/请求上下文条件，不能将固定客户端组合承诺为长期保证。

另有可靠性风险：`media.rs:152` 和 `media_platforms.rs:1007` 存在写死的抖音 Cookie/匿名标识兜底。未复制其值进报告，也未断言这些值已经失效；建议改为有明确寿命的匿名会话获取，失败时指导用户提供新凭证。

## 8. 修改顺序与验收门槛

1. 首先修 M01～M06：避免错误文件成功、恢复数据丢失、加密媒体误完成和无声默认下载；把本轮 5 个隔离复现转换成有断言的正式行为回归，并补多连接/暂停/重启情形。
2. 接通 B 站选集或收窄支持说明，再修代理、共享限速和可取消探测。兼容旧 MediaSelection/数据库 JSON，新增字段必须带默认值。
3. 补全网盘分页/截断提示，去掉伪造质量，区分配置连接数与实际连接数。
4. 逐站建立用户自有或授权的长期测试样本：公开/需登录、视频/多图、短链/长链、单轨/双轨、短视频/长视频、普通/过期直链。使用固定小文件和可控账号，不使用来源不明的付费分享。
5. 各站下载验收至少记录真实文件字节数、媒体轨道、播放/时长、暂停恢复、重复命名、取消、完整性；将网络风控、样本失效、权限限制与产品缺陷分别记录。

本报告没有给出“全站兼容通过”或“全部问题已修复”的结论。公网完整下载、登录/付费内容、真实直播长测、网盘大文件、组件安装和发布安装包验收仍未完成。
