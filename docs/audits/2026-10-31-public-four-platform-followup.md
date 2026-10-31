# 抖音、快手、小红书与西瓜：自主公开样本补充验证

日期：2026-10-31。根据用户要求自行寻找公开样本。下列初批研究发生在北京时间15:36—15:46；后续真实生产结果在本节索引单列，不把外部工具、页面200或格式列表冒充本项目完整下载通过。

初批研究没有修改生产源码，没有安装/更新工具、登录账号、读取个人浏览器Cookie或转存内容；后续实网发现的产品逻辑修复另由主任务实现并重新验证。工具为本机已有yt-dlp 2026.07.04、FFmpeg/FFprobe9.0；文件均在独立TEMP。新建匿名Edge上下文只正常访问网页，没有解决验证码、切换IP或执行登录绕过。

## 最终生产验收索引

- 统一round10实际结果为**1471通过、0失败、16显式忽略，97.61秒**，另Range **9通过、0失败，5.07秒**；274份Rust源码快照Changed=0，见[总台账](2026-10-31-remediation-status.md)。release round3、候选13项检查及原生EXE smoke仍待结果，不以测试EXE代替正式产物。
- 初始公开媒体11条输入4完整通过、7失败，两批均退出101的历史保留，见[公网结果](2026-10-31-public-media-followup.md)。小红书官方教程随后已由真实Manager再次探测并完成，58,715,162字节及SHA与下文独立oracle作外部实际相等比对通过，完整解码通过；普通笔记仍未通过。
- 修复抖音默认根Referer覆盖具体作品源的问题后，7694589931460594953和7408129200156101907各执行直连/显式环境代理，**4/4真实完整下载通过，exit0、80.01秒**；均验证AV、完整时长、全片解码和实际SHA，各样本两网络条件指纹一致。详细最终行由主任务补入同一[公网报告](2026-10-31-public-media-followup.md)与[机器结果](2026-10-31-public-media-results.json)。这4次仅复用2个既有URL，不是新增4个不同样本，不能合称15个不同输入。
- 以上修复后的抖音正例不会删除初批失败。快手、西瓜和小红书普通笔记仍按各自实际失败及访问条件记录；[诊断文案修复](2026-10-31-diagnostic-wording-fixes.md)只改善提示，不将失败本身变为成功。

## 初批实际观察（历史）

| 平台/范围 | 实际结果 | 下载层级与限制 |
| --- | --- | --- |
| 抖音 | 原有真实native公开测试在最新round7测试EXE副本中1/1通过，2.13秒；相同样本及另两个公开样本的已有yt-dlp均要求Fresh cookies | native请求及JSON转换取得非空标题/格式/HTTPS地址；尚不能据此宣称完整文件。该次自测发生在native codec真实性补充修复之前，没有把硬编码AAC当作已实测音轨。 |
| 快手 | 官方研究样本、上游公开问题样本以及当天官方热榜的两个新ID，已有工具均Unsupported URL；匿名浏览器对研究样本得到result=2、无媒体 | 没有完整文件；不能归因于全部是旧样本失效，也不能据200空壳宣称视频有效或下载通过。 |
| 小红书普通笔记 | 三个上游公开测试/问题样本均No video formats；旧样本HTTP最终跳/404，匿名浏览器对分享样本明确显示IP风险安全限制300012/300031 | 没有完整笔记视频；风险页与删除/失效不能混同，不能笼统写成已确认要求登录或只需Cookie。 |
| 小红书官方教程 | 官方页面HTML5MediaEmbed提取成功；已有工具匿名完整下载58,715,162字节，FFprobe双轨和全片严格解码通过，21.31秒 | 这是独立工具oracle，正式生产Manager同样本完整验证尚由主任务执行；不替代普通explore/discovery笔记适配。 |
| 西瓜 | 上游样本与另一个可索引创作者样本，工具无Cookie均要求Cookie；匿名浏览器最终进入官方/app下载页，无video | 全新匿名会话Cookie回交工具后变为SSR JSON解析失败，仍无完整文件；不能报告为“取得Cookie即可下载成功”。 |

## 样本的实际来源

- 抖音基线：[项目已有公开样本7694589931460594953](https://www.douyin.com/video/7694589931460594953)。生产native自测直接调用真实fetch与转换，断言desc、formats及HTTPS地址非空。
- 另选[kennyS公开创作者页面7408129200156101907](https://www.douyin.com/video/7408129200156101907)，主站可索引标题自述唯一官方账号；没有据此假定当前机器匿名可播放。另用[yt-dlp上游Douyin公开测试](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/tiktok.py)中的6961737553342991651对比现有工具回退。
- 快手研究样本[3xwwuqndapzs6nu](https://www.kuaishou.com/short-video/3xwwuqndapzs6nu)由[快手知识工程组Kuaipedia](https://github.com/KwaiKEG/Kuaipedia#data)直接提供。该研究仓库为CC BY-NC-SA4.0；不把仓库许可外推为所有快手UGC的授权。另有[上游公开问题14024](https://github.com/yt-dlp/yt-dlp/issues/14024)所列3x25msbsnncn8cs。
- 为避免只试历史ID，还从当日实际请求的[快手精彩页](https://www.kuaishou.com/brilliant)Apollo热榜取得3xfd8shgpc49zsw、3xiypede7wnhc2m，再分别执行现有工具探测。未臆造ID，未调用私有GraphQL接口。
- 小红书使用[yt-dlp上游XiaoHongShu测试](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/xiaohongshu.py)的6411cf99000000001300b6d9、674051740000000007027a15，以及[公开问题15467](https://github.com/yt-dlp/yt-dlp/issues/15467)的6956513d000000001e022eda。分享参数只放私有TEMP清单，报告不公布token。
- 西瓜使用[上游Ixigua公开测试](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/ixigua.py)的6996881461559165471，另找[创作者原页面7355478885854773823](https://m.ixigua.com/video/7355478885854773823)，标题“牙齿的数量大对比，这个生物居然有3000颗牙齿？”。页面可被索引不证明当前机器取得实体。

## 官方小红书教程的独立文件oracle

公开来源：[云服务AI工作台使用指南](https://miniapp.xiaohongshu.com/doc/DC431239)。普通HTTP实测200、真实video标签，匿名yt-dlp返回HTML5MediaEmbed/format0/MP4。**原始探测duration、音视频codec及大小均未知，保持原结果，不伪造产品元数据。**

实际媒体HEAD为200、video/mp4、支持Range，Content-Length=58,715,162。随后在独立TEMP用已有工具执行一次完整下载，最大256MiB、下载120秒期限，再对整个文件进行FFprobe及FFmpeg -xerror完整解码。

| 项目 | 实测 |
| --- | --- |
| 实际文件长度 | 58,715,162字节，与先前HEAD精确一致 |
| SHA-256 | a4c0ce31f9e4a3f894c03ef508f34a556775b5010ecb473e0d918cee6a0e7efc |
| 实际容器时长 | 287.300000秒 |
| 视频 | H.264、3840×2160、287.300000秒 |
| 音频 | AAC、287.299002秒 |
| 完整解码 | FFmpeg -xerror对全片执行，退出0 |
| 总耗时 | 21.31秒，包括下载/指纹/探测/完整解码 |

此SHA是实际文件指纹，不是站点官方校验值。主任务可将该文件作为明确的测试oracle，让实际生产Manager再完整获取并比较size/SHA/时长；不将oracle数值塞回probe响应，不放宽生产自动选择未知音轨的规则。

## 匿名浏览器与Cookie的范围

每个平台均使用全新的Edge临时上下文；没有点击登录或接触日常用户配置。只将该上下文正常访问所产生的匿名Cookie写到私有TEMP jar，重新交已有工具做一次对比：小红书仍NoFormats、快手仍Unsupported、西瓜仍解析失败。Cookie值不写报告/日志，不导入客户端凭证库。

小红书截图保留原文“当前笔记暂时无法浏览”“IP存在风险，请切换可靠网络环境后重试”，未按页面建议切换网络绕过；快手JSON只有result=2，不能自行推断具体风控原因。西瓜仅可确认普通匿名浏览最终到了官方APP下载页。

## 私有证据索引与后续

- 第一批7次现有工具metadata：%TEMP%/maobu-four-platform-public-cc_7xzar/summary.json。
- 页面状态/HTML/快手当日热榜：%TEMP%/maobu-public-page-evidence-i2jgjwei/。
- 快手研究/上游例与新小红书例：%TEMP%/maobu-public-samples-second-tq4heuqi/summary.json。
- native抖音实际自测：%TEMP%/maobu-native-douyin-e74f3542959148669d57a8f69c2c2535/result.log。
- 真实匿名Edge截图/DOM结果：%TEMP%/maobu-public-browser-OkSPjF/results.json。
- 全新匿名Cookie对比及生产测试清单：%TEMP%/maobu-public-anonymous-cookie-probe-8c3_5h6_/。
- 官方教程完整文件、独立oracle及探测：%TEMP%/maobu-official-public-doc-video-d_6mps3o/download-feasibility.json。

主任务后续已使用显式opt-in的真实生产probe/Manager清单，完整结果见本报告开头最终索引。未知轨道在测试清单明确手选，Manager按实际桌面提交路径再次探测原页面并取得同ID直链，随后由真实下载内核完成；前期play-0交网页工具回退或缺AppState的夹具问题没有混算为最终网站失败。当前仍不能宣布这四个平台全部适配成功，且没有新增整个原生GUI公网闭环已验证的承诺。
