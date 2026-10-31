# 用户授权 B 站完整视频验收

日期：2026-10-31。样本：[BV1VeHQ6tEaS](https://www.bilibili.com/video/BV1VeHQ6tEaS/)，标题“超市生存挑战后续！4人吃完整个超市赢100万美金！”。仅记录 canonical 地址，不记录追踪参数或签名 CDN URL。

## 实际结论

**该样本匿名完整下载、音视频合并、全时长校验及全片解码通过**。不是截取片段，也不是仅返回格式列表。

生产 `probe_for_task` 与 `download_with_manager` 在独立单 Wry 上下文、独立 Store/TEMP 下实际执行；没有安装/更新工具，没有读取个人数据库凭证或浏览器 Cookie。使用本机已有 `yt-dlp 2026.07.04`、`FFmpeg/FFprobe 9.0-full_build-www.gyan.dev`。代码只在显式 `MAOBU_TEST_NATIVE_ONLY=bilibili-sample` 且 `MAOBU_TEST_BILIBILI_SAMPLE=1` 时运行，默认测试不连接该公网样本。

选择最低 360P、已声明实际音轨的分离组合 `100022+30216`，完整下载至 TEMP，整体 600 秒截止与 1GiB 工作文件保护。C 盘当时约 195GiB 可用；本轮实际耗时 **64.87 秒**，1 通过、0 失败、0 忽略。

| 检查 | 实际结果 |
| --- | --- |
| 元数据时长 | 3465.694 秒，约 57 分 46 秒 |
| 实际最终容器时长 | 3465.694331 秒 |
| 视频轨 | AV1，640×360；轨道时长 3465.633313 秒 |
| 音频轨 | AAC；轨道时长 3465.694331 秒 |
| 最终文件 | 143,358,482 字节，约 136.72MiB，MP4 |
| 完整性 | FFprobe 确认双轨/时长，FFmpeg `-xerror` 对全片完整解码通过 |
| 身份条件 | 匿名；未提供个人 Cookie，没有绕过 DRM 或关闭站点验证 |

SHA-256 为实际流式读取到的源轨道与最终输出指纹；它们不是站点发布的官方校验值，不能冒称与站点提供的 SHA 比对通过。

| 文件角色 | 实际字节 | 实测 SHA-256 |
| --- | ---: | --- |
| 视频原轨 `100022` | 113,061,057 | `3f097f933f627b8991f44ae7141fb6d9f8ded8f24d70da344da5cc0dcd51039c` |
| 音频原轨 `30216` | 28,407,116 | `f794d2100b823a62aee951288d537e1fc82d14bda889ccf23b44cfb217348a72` |
| 合并输出 | 143,358,482 | `3f5048e42afbcf0bccbd034b61ab745d49bbedffd50b676996314be59016bb6a` |

源轨道和合并 MP4 的指纹不同是转封装的正常结果；验收同时检查两个源轨、最终轨道与完整时长，不要求不同容器文件字节相同。合并阶段保留原始轨道，成功提交后的清理由任务管理器负责。

## 这条真实输入发现并修复的问题

1. 用户链接具有 `zh-Hans` 语言路径。已有 July 工具对它走 Generic，虽然退出 0，却仅得到 `id=player`、空格式和未知时长；canonical 则正确返回 BV ID、57:46 和 15 个格式。解压原页面 gzip HTML 后，HTTP 200、标题和同一 BV ID 均得到确认。原 Generic 标题是 canonical 标题加站点后缀。
2. 新增窄正规化：仅已验证的 HTTPS 主站、精确语言前缀和合法 BV 路径转为 `/video/`，保留分 P、查询业务参数和 fragment；不猜测其他语言路径/子域。探测、下载与桌面 URL 预览均接线，纯 helper 回归覆盖合法及无关输入。
3. 原 parser 用 `get(filesize).or_else(get(filesize_approx))`，当 `filesize:null` 时错误地阻断估计值。真实样本的所有相关 size 都属于这一情况。现先解析合法精确值，再取有效估计值；新增安全默认 `file_size_approximate=false`，TS 可选兼容旧 JSON，界面明确标“约”。合并容器的计算总量也标估计。
4. 该样本预测视频 113,054,403、音频 28,398,763 字节，与实际分别不同。因此没有把估计长度当作严格文件完整性断言；最终长度、SHA、实际时长、双轨及完整解码单独核验。

普通 Python 主站 GET 与 yt-dlp 官方 API 的结果分别登记：原语言页面 200、canonical 普通 GET 曾 412；同一台机器的 yt-dlp canonical 匿名 API 连续成功。没有把网页 412 错写成所有解析失败，也没有把元数据成功直接写成下载成功。

## 证据与边界

- 完整生产路径日志：`%TEMP%/maobu-bilibili-user-sample.log`。
- 原/正规路径与公开页面证据：`%TEMP%/maobu-bili-route-evidence-bxvubuh8/summary.json`、`route-verification.json`、`page-verification.json`；私有 TEMP 中的原始元数据没有提交仓库，也没有公布签名 CDN 地址。
- 代码：`media/bilibili_sample.rs`、`sample_validation.rs`、`media_platforms/url_normalization.rs`、`media/format_size.rs`；生产输入选择仍依实际结构化清单，测试源轨核验不参与生产目录猜测。

这次只证明该日期、该样本、匿名 360P AV1/AAC 在当前机器及工具版本上的完整路径。不能外推为全 B 站、会员/地区限制、登录清晰度、番剧、多 P、字幕或直播均已通过。此前其他公开多 P 样本 412 仍保留原始记录。所有视频样本文件都在隔离 TEMP，不进入安装包或仓库。
