# 五类网盘：自行查找公开样本后的真实在线验收

日期：2026-10-31，Asia/Shanghai。此次使用网上项目作者主动发布的合法分发链接，不再要求用户提供样本。共检查 **13 个不同公开分享**，覆盖 PikPak、百度、夸克、123、蓝奏；没有读取个人账号、浏览器 Cookie 或用户数据库，没有登录、转存、破解验证码或执行下载内容。

本轮已补齐 **1 个有明确前置条件的公网文件完整下载与作者 SHA-256 校验**：蓝奏 Umi-OCR 的最终地址先由隔离匿名浏览器正常导航取得，再交真实 Manager 下载。**桌面直接粘贴蓝奏分享并自动经过 JS 中间页、真实扩展接管这两个链路仍未证明。** 其余四盘没有完整实体通过。生产列表、静态取链、浏览器自然下载地址、完整文件和 Manager 完成分别记录，不能互换。

## 实际结果

| 平台 | 自行找到的第一方发布来源与分享 | 当前真实生产列表 | 当前真实取链/实体 | 结论及条件 |
| --- | --- | --- | --- | --- |
| PikPak | [ZenFile 作者仓库](https://github.com/l930203811/ZenFile)公开的 `VOxGdQB3fVNO32sq_I3o2Wkmo2`；最小更新日志 Markdown **1,858B** | **64 文件**，成功 | 修前取链返回 Ok 空字符串，随后实体构造失败；修后明确拒绝空直链。再次只读诊断为 HTTP200、share_status=OK、medias=0、links 为空、web_content_link 为空，无其他实体地址 | **列表通过，真实取链/下载未通过**；上游没有返回可用地址，未臆造为登录或验证码错误 |
| PikPak | [词库项目作者仓库](https://github.com/SteveSuv/remix-words-funny)公开的 `VOEs95bTB0KGAg75t0Nrs-oOo1` | **1 文件**，`wordsfunny_data.zip`，**74,253,362B** | 同样无有效下载候选。唯一额外 HTTP 链接来自 `apps[0]`，id/name 为 `decompress-share`；有界 Range 返回200而非206，立即放弃 body，未当作压缩包下载 | **列表通过，真实取链/下载未通过**。得到许可后已取消人为8MiB整片限制作为候选，但仍没有实际文件源可交 Manager |
| 夸克 | [StarlightGUI 作者仓库](https://github.com/OpenStarlight/StarlightGUI)公开分享 `ee7a29ca2a76`，按作者公开提取码填写 | **15 文件**，成功，选择 `使用说明.txt` **1,962B** | 生产取链返回有效 HTTP 地址，带其返回的必要 UA/Referer 实体请求得到 **HTTP412**，没有 SHA | **列表/取链阶段通过，实体下载未通过**；不能把签名地址存在视为下载成功 |
| 夸克 | [ZenFile 作者仓库](https://github.com/l930203811/ZenFile)公开分享 `e6081a88d463` | **105 文件**，成功，最小日志 **2,976B** | 生产取链有效，实体仍为 **HTTP412** | 与前一作者样本一致；没有添加账号 Cookie，也没有绕过风控 |
| 百度 | [ZenFile 作者仓库](https://github.com/l930203811/ZenFile)公开分享 `1kYSfzTriRXwQPRL_c5Awig`，按作者提取码填写 | **223 文件**，成功，最小 Markdown **1,637B** | 生产取链明确要求登录凭证 | **列表通过，取链/下载受账号条件阻断**；本轮不登录、不转存 |
| 百度 | [Lumi 作者官网](https://huangder.top/)公布的 `1vDdFsoqQuuZlntUTUjssZw`，按官网密码填写 | **14 文件**，成功，最小 APK **27,828,652B** | 生产取链明确要求登录凭证；未下载或运行 APK | 与 ZenFile 分开实测，不能据公开可列出文件声称匿名可下载 |
| 百度 | [MyBox 作者仓库](https://github.com/Mararsh/MyBox)公布的 `1fWMRzym_jh075OCX0D8y8A` | 当前目录接口拒绝，未返回部分成功 | 未取链、未下载 | 当前样本权限/有效性或接口适配边界仍未解决；不归因为整个百度站点故障 |
| 123 | [JumpVideo 作者 README](https://github.com/GMYXDS/jumpvideo-electron/blob/main/README.md)公布的 `4neA-a9uUH` | **3 文件**，成功，Chrome 扩展包 **75,425B** | 生产取链返回业务 **5112**，要求登录或付费权限 | **列表通过，取链/下载受分享条件阻断**；未执行扩展包或安装客户端 |
| 123 | [ScreenFilter 作者仓库](https://github.com/cjyyx/ScreenFilter)公布的 `Be4Hjv-fUUtv` | **1 文件**，成功，APK **9,592,193B** | 生产取链仍为 **5112** | 第二个独立作者样本也未通过匿名取链；未下载/执行 APK |
| 123 | [OneList 作者仓库](https://github.com/msterzhang/onelist_app)公布的 Windows 分享 `1GzA-nL2PH` | 当前列表接口拒绝 | 未取链、未下载 | 历史作者链接不等于当前仍有效，保留失败证据 |
| 蓝奏 | [Umi-OCR 作者 README](https://github.com/hiroi-sora/Umi-OCR/blob/main/README_en.md)公布的 `https://hiroi-sora.lanzoul.com/s/umi-ocr` | 修复后曾完整列出 **2 文件**；103,389,593B 是早期最小项的人类可读大小换算，不能当作本次指定 Paddle 文件的精确长度；后一次第2页zt4保留 | 指定 `ivZ6e2rmu0bi` 静态取链后实际落到200 HTML中间页；临时匿名浏览器一次自然导航触发最终文件地址并立即取消，真实 Manager 随后完整下载 **134,293,725B**，作者 SHA-256 一致 | **浏览器取得最终地址→真实 Manager 完整文件通过**，134.79秒；不代表分享粘贴自动取链或扩展接管已通过。HEAD发送失败、GET200/HTML和服务拒绝均保留为阶段证据，详见下文 |
| 蓝奏 | [Lumi 作者官网](https://huangder.top/)公布的 `b0xxfmxvi`，已填官网密码 | 页面无验证码/挑战；同生产请求器与静态参数，第1页真实 **5 文件**，第2页zt4，完整列表不算通过 | 真实服务端info为“请刷新，重试0”，不是已经证实提取码错误 | **服务拒绝，停止本轮请求**；没有添加猜测延时/刷新循环，也没有将第一页5项当完整目录 |
| 蓝奏 | [LanZouCloud-API 作者旧回归链接](https://github.com/zaxtyson/LanZouCloud-API#v255) `ic59zpg` | 生产函数返回1条但大小0，不能确认真实文件元数据 | 未把未知大小/可能失效页面当文件下载 | 仅记历史样本形态观察，**不计有效列表或下载通过** |

## PikPak 本轮实际修复

原 `resolve_pikpak_file` 对 `medias[0].link.url`、原文件和顶层 `web_content_link` 只检查 JSON 字符串类型；空字符串也会截断 fallback 并返回 Ok。本轮两个作者公开分享都实际复现了 Ok 空地址，属于产品错误处理缺陷，而不是依靠 fixture 推断。

修复拆入 `pikpak/direct_link.rs`：先过滤有效文件对象；媒体候选保持服务端顺序逐项 trim、跳空，再回退原文件及顶层地址。全部空值明确中文失败；最终只接受 HTTP(S)、有效 host、无嵌入用户名/密码、无控制字符的 URL。遇非空恶意地址直接拒绝，不能静默跳过后伪装成功。没有更换 API、模拟账号或改变分享权限。

相称回归涵盖首媒体空而原文件有效、首媒体空而后项有效、file_info 为 null 时回退 file、全部空、空白、非 HTTP 协议、内嵌凭据和控制字符；真实本地 HTTP 经过生产 captcha/file_info 请求器，首次实体410后同一文件重新解析得到新地址，读实际内容并校验 SHA，最后全部空必须失败。**这些是生产解析/刷新再查询路径的本地证据，不是五盘线上通过，也不冒充 Manager 自动刷新完整 E2E。** 3项测试已在最终Round10全套实际通过，详见下节。

修后真实 ZenFile 最小1,858B和词库74MB均重新执行。业务响应200/OK，真实 `medias_count=0`，`links_keys=[]`，`web_content_link` 空；没有 data 包装中的有效地址，也没有被首个空媒体掩盖的后续地址。词库的云端解压应用入口不是实体 URL。此时应准确报无有效直链，不能虚构为登录问题或下载成功。

## 蓝奏本轮实际分因与最小修复

Umi-OCR 作者页面实际 HTTP 可达、无密码/验证码/WAF，目录参数包括 fid/uid/puid/t/k/pg；生产等价匿名表单第1页返回zt1、2个文件，第2页返回 **zt2、text=`no file`**。原完整性保护没有识别这项机器终页值，误把正确结束当目录拒绝。只追加此精确终页值，仍要求zt2，保留错误密码、访问拒绝、第二页错误和无进展失败；本地真实HTTP新回归要求完整2条且只请求2页，不返回部分成功。

文件页只有一个实际下载 iframe，nonce/sign/各表单字段都能静态识别。当前 iframe 用 `url:dom_ajaxs`，通过明确 `typeof(killdnsweb)=='undefined'` 默认分支选择 `domain2`；真实地址为官方 **apifile.lanzouw.com /ajaxfile.php**，参数含file。旧解析只接受字面量，相同表单被发往本站 `/ajaxm.php` 而失败。独立只读比对保持相同nonce/sign及原kd=1，只改为页面声明端点，就真实返回zt1、dom/url非空，证明是本项目端点选择缺陷。

已拆出 `lanzou/endpoint.rs`，只静态识别字面量、已声明的直接变量和上述明确默认分支，不执行网页或外部JS。同源路径保持兼容，跨域仅允许这两个官方HTTPS API域：apifile.lanzouw.com、apifile.woozooo.com；拒绝未知动态地址、第三方域、内嵌凭据、非HTTP(S)和控制字符。本地HTTP完整经过生产文件页、iframe、官方API路由和实体，另覆盖字面/直接变量/默认分支及负向地址。endpoint 2项、pagination 5项已在最终Round10全套实际通过，详见下节。

之后Umi/Lumi的完整目录重验出现第2页 **zt4、info“请刷新，重试0”**；无密码的Umi也出现此状态，不能继续解释成密码错误。该轮随即停止，错误增加页码/状态码而不回显上游敏感正文；没有增加猜测重试策略。此前生产列表2条成功、独立正确端点zt1、末次服务拒绝是不同阶段的实际证据，全部保留；后续指定文件补测另列，不能用其中一轮覆盖另一轮。

指定文件HEAD复验：release3构建开始后自然冷却，**16:22:05**只调用一次最新生产 `resolve_lanzou_file`，跳过目录重扫、额外schema和任何重试。生产取链成功返回有效HTTP地址；随后HEAD在30秒请求限时下发送失败（`error sending request`），本次总36秒。该轮未发Range、未显式读取或保存实体body，没有真实HEAD/CR总长或整片SHA可报告，随后停止该轮请求，没有改生产源码。证据为 `%TEMP%/maobu-public-cloud-single-umi/results.json`，同样已并入持久脱敏JSON。

随后追加一次明确GET对照：**16:35:00**，不重扫目录、不发HEAD、不重试，路线明确为 `default-existing-env-proxy`，不切换节点或添加登录凭证。最新生产取链成功后，先在隔离TEMP保存签名地址及必要匿名UA/Referer，再只发送 `GET Range: bytes=0-65535`。实际6秒内得到 **HTTP200**；依照严格条件立即drop响应，未读取大body，因此没有206、有效Content-Range总长或64KiB SHA可记为通过。失败阶段为 **GET_HEADERS**。这个结果证明GET已获得响应，与HEAD发送失败不同；它既不证明普通GET整片失败，也不是完整实体成功。私有参数仅 `%TEMP%/maobu-public-cloud-single-get-umi/lanzou-private-download.json`，总长仍null；脱敏结果同目录 `results.json`。该轮随后停止，以下有界内容识别和浏览器导航是分开的后续观察。

## 蓝奏指定 Paddle 文件：有条件的完整下载通过

指定文件为公开 Umi-OCR 分享中的 `ivZ6e2rmu0bi`，浏览器实际建议文件名 `Umi-OCR_Paddle_v2.1.5.7z.exe`。后续有界读取确认静态取链的 `developer4.lanrar.com` 返回 `text/html`，gzip体2,714字节、解压4,321字节，是中间文档而非安装文件。HTTP200没有被计为文件成功。

**16:42:26**，独立临时匿名 Chrome 只进行一次正常导航，沿用现有环境代理和实际 UA；未登录、未操作验证码、未切换节点或注入页面脚本。浏览器自然触发 `exe2.webgetstore.com` 的二进制下载，响应200、`application/octet-stream`，报告总长134,293,725字节。CDP已设为拒绝下载，事件最终 `canceled`、`receivedBytes=0`，因此这个浏览器步骤只证明最终地址被自然取得，不能计作完整下载。四个临时匿名 Cookie 的域仅为 `developer4.lanrar.com`/`.lanrar.com`，均不匹配最终下载域，没有跨域转发；没有将未观察到的最终请求 Referer 编造成必要条件。

最终地址通过私有 TEMP 清单交给实际 Manager 普通 HTTP 任务：只使用实际 UA，无 Cookie 或猜测 Referer；单连接、1MiB/s、无重试、4分钟任务限时。测试进入真实队列/worker/发布与停止屏障，将作者 SHA-256 设置为任务校验值，并对发布后的文件再次独立计算长度和指纹。**1/1实际通过，134.79秒，134,293,725字节**，SHA-256 为 `abe00248d6e68dda75029739751ad7258f0ecc14d720b19240a844f5341859b2`，与[作者 v2.1.5 发布页的 Paddle 校验值](https://github.com/hiroi-sora/Umi-OCR/releases/tag/v2.1.5)一致。下载保存为隔离 `.bin`，`executed=false`；未运行或安装文件。早期103,389,593B的列表换算不属于此文件，不能替代本次精确长度。

此证据只证明 **浏览器自然取得最终下载地址→真实 Manager 完整文件及作者校验**。静态分享解析后自动经过中间页、原生界面粘贴分享、真实扩展拦截并交给 Manager，仍未完成端到端验收；也未证明该临时地址长期有效或过期后自动刷新成功。不能据此把整个蓝奏适配或五盘下载标为全绿。

脱敏结果已并入下方持久 JSON。原始证据位于 `%TEMP%/maobu-lanzou-browser-single-j2vHzX/private-browser-manifest.json` 和 `%TEMP%/maobu-cloud-manager-file-e4ea808114ad45149cb08fac7e937947/manager-file-result.log`；前者含临时签名参数，仅供本机复核，不复制进报告或仓库。

## 已完成的本地回归阶段

根任务Round10已实际退出0：**Rust lib 1471 passed / 0 failed / 16 ignored，97.61秒；Range集成9 passed / 0 failed，5.07秒**。包含当时08:13 UTC源码的PikPak direct_link 3项、蓝奏endpoint 2项、pagination 5项；274个源文件快照在该门禁期间Changed=0。日志为 `%TEMP%/maobu-final-cargo-round10.log`，本报告已核对其中实际test result。此为该轮云盘源码的历史阶段证据；后续启动错误处理补强已改变整体源码，最新统一门禁由[总台账](2026-10-31-remediation-status.md)记录，不能用Round10代替。

这些已通过的本地行为回归证明候选筛选、端点解析、正常终页和错误保护，并不把16个忽略用例计成功，也不改变上述公网账号/服务/实体条件。

## 可复现方法与证据

新增独立脚本 [run_cloud_public_probe.ps1](../../scripts/audit/run_cloud_public_probe.ps1) 和 [cloud_public_probe.rs](../../scripts/audit/cloud_public_probe.rs)，导入当前五盘、CloudRequestContext、分页与 Cookie 作用域生产源码，不启动 Tauri setup，不连接数据库。仅将展示层错误脱敏函数换为独立脱敏；实际网络、列表与解析没有 mock。依赖从最近真实应用测试的 Cargo fingerprint 选择一致图，避免混用不同编译特性的 rlib。

输入为公开来源的 JSON 数组：platform、url、source、可选公开 password。输出目录应位于独立 TEMP。脚本单分享90秒总限时、普通响应4MiB限制；独立小文件实体检查8MiB，已获准较大候选只做HEAD及最多64KiB Range，把真正整片下载交给真实 Manager。任何HTML实体、错误状态、错长度或超量字节都不记成功。它不能替代 Manager 完成状态、发布和重启验收。

经脱敏的持久证据：[cloud-public-results.json](2026-10-31-cloud-public-results.json)。本轮批量在线阶段约15:38至16:11，指定文件HEAD复验为16:22、GET对照为16:35、浏览器自然导航记录为16:42:26；完整 Manager 运行另由日志记录134.79秒，不伪补缺少的精确起止钟点。早期两轮旧脚本只检查函数 Ok，PikPak 的该值已明确注释为“返回Ok但未经有效URL校验”，并按后续真实证据更正有效取链结论，保留历史现象，不掩盖先前测量不足。

临时签名直链和请求参数只留在隔离 TEMP，不写入仓库或报告；没有完整下载样本、账号凭证或工具可执行文件进入提交。初审10:19的旧列表结果仍是历史记录，不能据此覆盖本轮新样本现状；同样，后端源码修复也不会自动把线上限制改写成通过。
