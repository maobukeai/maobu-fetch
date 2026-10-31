# 自主联网查找后的媒体公网完整验证

日期：2026-10-31，Asia/Shanghai。用户要求自行联网查找样本后，使用公开作者页面、平台官方资料及[yt-dlp官方测试来源](https://github.com/yt-dlp/yt-dlp/tree/2026.07.04/yt_dlp/extractor)寻找可复现输入。没有读取个人账号、浏览器Cookie或真实任务数据库，没有登录、解验证码、为绕过验证轮换节点或绕过DRM；部分样本显式使用既有环境代理，未检查出口IP。没有安装/更新媒体工具。

## 真正执行的链路

新增入口全部为`cfg(test)`：同一无窗口Wry实例注册私有Manager；每个样本使用独立任务，生产probe、真实队列/worker、再次探测源页面取得同一选定格式、传输、发布与数据库完成状态；随后FFprobe检查双轨/时长、FFmpeg严格全片解码、读取实际文件SHA。没有用合成服务器代替本节的公网来源，也没有把单独工具元数据成功记为下载成功。

清单需显式`MAOBU_TEST_PUBLIC_SAMPLES=1`、`MAOBU_TEST_NATIVE_ONLY=public-samples`和本地清单路径。默认测试不执行这些公网样本。每条正常操作限4分钟、连接限30秒、任务1MiB/s；声明及可观察进度超过256MiB则取消，媒体目录另有监视。检查点/停止存在延迟，因此不把它描述为所有磁盘布局上字节不差的硬256MiB上限。超时给予35秒等待，之后统一经过真实30秒worker退出屏障，清理失败停止整批；下一样本不会沿用前一任务的活动连接。

未知音轨/轨道只在清单明确选择时允许继续，生产probe字段保持未知；最终双轨与解码必须真实通过。代理样本显式使用已配置环境代理，凭据只留私有进程/临时数据，不写报告。URL展示去除签名查询和userinfo，保留YouTube公开视频ID。

首批11条输入分两批执行，**4条完整通过，7条未通过；两批测试均真实退出101**。抖音另有修复前2次代理对照失败、修复后4次完整通过。共**17次Manager执行、11个不同URL，8次完整成功/9次历史失败；按不同URL计6个已完整成功**，不能把重复路由写成8个不同视频。保留“任何样本失败即整批失败”的断言，没有为了全绿删除负例。逐条机器结果见[public-media-results.json](2026-10-31-public-media-results.json)。用户B站整片是此前独立成功项，不混入上述计数。

## 完整通过的实际文件

| 输入及条件 | 实际结果 | 输出SHA-256 |
| --- | --- | --- |
| [微博公开测试视频](https://m.weibo.cn/status/4189191225395228)，匿名直连、mp4_ld | 真Manager完成；1,126,415B；H264 360×360＋AAC；53.173991秒，元数据53秒；全片解码；6.47秒 | e69ecdaef96d71dcc8b8bee467a432088090da906a310502e62f0e442d5f7d02 |
| [Brooklyn Nets官方媒体发布](https://twitter.com/BrooklynNets/status/1349794411333394432)，匿名、显式环境代理、手选未知音轨候选http-288 | 修复Twitter空native丢弃工具候选后，真Manager完成；2,664,147B；H264 480×270＋AAC；324.501333秒，元数据324.484秒；全片解码；14.49秒 | 71431b4e9eb32d59d89b35d941f1d4235cc90427629c4cf13d747859277694b0 |
| [Hank Green公开TikTok测试输入](https://www.tiktok.com/@hankgreen1/video/7047596209028074758)，匿名、显式环境代理 | 真Manager完成；1,790,611B；HEVC 576×1024＋AAC；21.268005秒，元数据21秒；全片解码；10.07秒 | c3352e4c1ebe4f7b5898121137caa23b616c341ec47c9f224a3c798bfc550f7e |
| [小红书官方云服务教程](https://miniapp.xiaohongshu.com/doc/DC431239)，匿名直连、手选HTML5格式0 | 真Manager完成；58,715,162B；H264 3840×2160＋AAC；287.3秒；全片解码；81.40秒 | a4c0ce31f9e4a3f894c03ef508f34a556775b5010ecb473e0d918cee6a0e7efc |

这些SHA是本次实际指纹，不是平台发布的官方校验值。小红书教程probe原duration/codec仍为未知；287.3秒来自独立完整下载的oracle，清单仅显式传入时长，不伪造产品元数据。Manager输出的字节数和SHA随后又与独立oracle作实际相等断言通过，证据为临时目录`official-doc-oracle-comparison.json`。**该官方教程属于通用HTML5网页，不代表普通explore/discovery笔记通过。**

界面源码允许“分析媒体→未知音轨警告→明确手选→提交”，不能称默认一键有声。此节用真实Manager再次probe路径，未操作原生GUI；页面与组件验证见[四平台补查](2026-10-31-public-four-platform-followup.md)与[完整UI证据](2026-10-31-full-ui-validation.md)，不能合并声称一次真实GUI公网闭环。

## 抖音真实逻辑修复后的完整复测

根因是`inject_media_credentials`自动添加`https://www.douyin.com/` Referer后，旧代码把Referer覆盖为探测源，丢失用户实际作品ID；该错误被吞后走工具，才出现Fresh cookies失败。旧错误并未证明CDN403。新的小型`media_source`模块始终优先当前作品URL，仅对精确已知CDN按同平台、HTTPS及具体作品校验恢复原始来源，已存来源优先于任意Referer。8项源码行为回归及最终round10整套通过。

另以生产HTTP adapter做12个HEAD/GET0-0/GET0-65535、直连/显式代理对照，均为200或精确206，排除单字节Range、普通UA/Referer和代理差异的错误归因。没有更改网络策略或新增CDN盲目兜底。

`douyin-source-fixed.log`实际**4/4通过、exit0、80.01秒**：两个样本各跑直连与显式环境代理，均经过真实Manager再次probe、队列、worker、发布/完成、双轨、完整时长及严格全片解码；同一个样本两条网络路径的字节数和SHA一致。

| 样本 | 实际字节与轨道 | 全片时长 | 实测SHA-256 |
| --- | --- | --- | --- |
| 7694589931460594953 | 9,921,329B，H264 1922×1080＋AAC | 35.766667秒，元数据35.767 | abc97e15776b15cead6dbe5e456d4fb2279e35d8ad9bc120a5b2d9e67332260d |
| kennyS 7408129200156101907 | 16,340,971B，H264 1080×1920＋AAC | 60.666667秒，元数据60.667 | 25522797bee906aee4984f4c2b99ce7d73d9ad08c60c38b36b085f9d15fd3377 |

直连耗时分别12.72/20.00秒，代理18.54/28.72秒；这是本次具体条件，不能推广为速度基准。没有个人Cookie，也没有把未知probe codec伪造为已确认，测试清单明确选择后才凭实际最终文件确认双轨。完整源选择证据见[原生作品源修复](2026-10-31-douyin-source-fixes.md)。

## 首批7条失败的历史及当前边界

| 输入 | 实际达到的阶段 | 当前结论 |
| --- | --- | --- |
| [Blender Big Buck Bunny](https://www.youtube.com/watch?v=YE7VzlLtp-4)，显式环境代理 | 生产元数据25格式、597秒；实际传输被拒。独立已有yt-dlp选同格式18也在3.05秒返回HTTP403 | 该网络/工具/无凭证组合未完成；403本身不能证明PO Token、账号过期或软件无缺陷。影片有[官方CC BY许可](https://peach.blender.org/about/)，没有换成无权下载的付费内容或绕过校验 |
| [抖音公开样本](https://www.douyin.com/video/7694589931460594953) | 首批原生元数据成功但错误回退工具 | 后续源选择修复后直连/代理均完整通过，见上节；旧失败不擦除，也不算当前CDN403 |
| [kennyS官方抖音输入](https://www.douyin.com/video/7408129200156101907) | 同一源选择错误在第二来源复现 | 后续两网络路径均完整通过；不外推其他作品/图集/直播 |
| 小红书官方上游公开测试笔记674051740000000007027a15 | 生产元数据未得到格式；独立匿名浏览器给出IP风险限制 | 笔记未通过；没有解决/绕过安全页面，官方教程成功不替代此结论 |
| [西瓜创作者公开输入](https://www.ixigua.com/7355478885854773823/) | 生产元数据拒绝；正常匿名页面到下载App入口，新的匿名Cookie仍解析失败 | 未完整下载，不能写成只差登录或Cookie便可用 |
| [快手官方研究示例](https://www.kuaishou.com/short-video/3xwwuqndapzs6nu)，来源[KwaiKEG项目](https://github.com/KwaiKEG/Kuaipedia#data) | 工具Unsupported；主站匿名壳/JSON result2未给视频。另取当日两个新热榜ID仍Unsupported | 当前网页适配未通过，排除仅凭编造ID或单个旧链接下结论；不绕挑战 |
| [TikTok Patrox旧测试输入](https://www.tiktok.com/@patroxofficial/video/6742501081818877190)，直连 | 生产元数据失败；独立工具也Unexpected response | 仅该输入/网络失败；另一个公开输入在显式代理条件下完整通过，不能外推全站失败 |

旧错误文案曾把匿名拒绝叫“登录失效”，并对YouTube403给出无依据的PO Token/恢复时间承诺，已单独交付诊断文案修复；JSON保留当时观察，并注明不是当前正确诊断结论。后续对抖音CDN的HEAD/Range/Referer诊断和其他独立样本只按实际证据增补，不擦除失败历史。

## 证据位置与待验范围

统一临时根：`%TEMP%/maobu-public-media-audit-66916caf6df9411f84e976a5b256c740`。`positive-final.log`、`secondary-final.log`分别为131.87秒和32.91秒，两者退出101；相同新编译测试EXE的SHA为`bc59d8d88439d5128373bc2063e25dde318ce9778e9ce5e71c2df547063f327e`。前两轮夹具的直连选择、AppState未注册导致的失败另留TEMP，不当成产品线上失败。

已有yt-dlp 2026.07.04及FFmpeg/FFprobe9.0，没有更新工具。无真实登录、会员/地区画质、字幕、多P整批或长期直播验收；这里每条完整结果只对应具体样本、格式、时间与网络条件。其他平台的公网分层见[总矩阵](2026-10-31-platform-validation-matrix.md)，五盘另见[13个公开分享](2026-10-31-cloud-public-validation.md)。
