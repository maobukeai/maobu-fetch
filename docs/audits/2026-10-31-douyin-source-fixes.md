# 抖音原生任务源地址丢失：公网定位、修复与复验

日期：2026-10-31（Asia/Shanghai）。本轮由公开样本的真实队列下载失败触发；不把标题/时长解析成功或独立 HTTP 206 当成完整下载通过。

## 已证实的生产原因

初始公开样本 `7694589931460594953`、`7408129200156101907` 都能经正式 native probe 得到作品时长与一个格式，但进入真实 Manager 后错误提示认证失败。相同两条任务切到现有环境代理仍失败，不能据最初 direct 失败就归为网络或登录限制。

只读追踪实际生产代码后确认：`inject_media_credentials` 对没有存储凭证的抖音任务也会补浏览器 UA 和 `https://www.douyin.com/` Referer。接下来的 native source 选择却只要 Referer 包含平台域名就优先取 Referer，因而将具体 `/video/{id}` 替换成首页。首页没有具体作品 ID，正式 Douyin probe 返回错误；该错误被 `if let Ok` 分支折成没有 play_url，随后错误进入 yt-dlp 原网页路径并出现 Cookie/访问校验提示。该提示不能证明实际 CDN 拒绝、用户登录曾失效或需要补个人 Cookie。

## 有界匿名 CDN 证据

独立 rustc 探针直接使用现有生产 `media_platforms` fetch/convert。为保持原模块子目录解析，仅把未修改源码字节复制成 TEMP 模块树；未替换网络、解析或选 URL 行为。未调用的展示类型使用编译支架。所有实际签名 URL 与原始详情只保存在 TEMP 私有文件，不输出 URL、Cookie 或代理值。

第一轮共 **20 个实际 case**，使用原进程默认环境代理路径：两样本各两个实际 CDN 候选；首候选比较没有显式 UA/Referer、浏览器 UA、站点根 Referer、具体作品 Referer，另查备选。共 **10 个 HEAD=200、10 个 Range GET=206**，每次 GET 只读前 65,536 字节；同一样本所有头组合/两个候选的前缀 SHA 一致。该轮没有把默认环境代理误称为 direct。

第二轮直接导入正式 `http_redirect::send`、`request_headers`，使用与 Manager 自动注入相同的浏览器 UA、抖音根 Referer、无 Cookie。对相同 TEMP URL 显式比较 `.no_proxy()` 与当前已有 HTTPS_PROXY，两样本合计 **12 个实际 case**：两路 HEAD 均为 200，`Range: bytes=0-0` 均为 206 且实际 1 字节，`Range: bytes=0-65535` 均为 206 且实际 65,536 字节。两路前缀 SHA 相同；没有换到新代理、登录或处理新挑战。未重新生成 direct 出口的签名 URL，故该轮是同一候选的严格请求对照。

| 样本 | 正式元数据时长 | HTTP Content-Range 总长 | 64 KiB 前缀 SHA-256 |
| --- | ---: | ---: | --- |
| 7694589931460594953 | 35.767 秒 | 9921329 | 06f5fd07510246842b49dd73f21ff50c3948e78c28d4577bce131f9f81f8b4f1 |
| 7408129200156101907 | 60.667 秒 | 16340971 | 5bc71339df651b2724ef893450fb1a45d49b66a7909fbc6f0d22115021d31b9b |

第一轮手动逐跳记录中，实际候选均直接在 `v5-dy-ov-experiment.zjcdn.com` / `v3-dy-o.zjcdn.com` 返回，没有跳转。第二轮使用生产 redirect adapter，只记录最终响应，不伪称对其内部增加了逐跳 trace。以上证据排除这次样本中的“只禁 HEAD”“拒绝单字节 Range”“缺 Referer”“首个候选坏”“仅代理可达”等解释；它们仍不是完整媒体下载验收。

脱敏证据分别是 `TEMP/maobu-douyin-cdn-diagnostic-9cc9fc04b3774a81b3514b9bdaeeaa38/sanitized-results.json` 与 `TEMP/maobu-douyin-worker-diagnostic-1469a9838c4b40ffb29085970462346b/sanitized-worker-results.json`。脚本为 `run_douyin_cdn_probe.ps1` / `run_douyin_worker_probe.ps1`；单体等待与请求数、正文前缀均有边界，不下载整片或读取个人账户数据。

## 必要修复与本地回归

新增 `manager/media_source.rs`，Manager 原分支只调用小辅助函数：原任务源地址始终优先，不由根 Referer、同站别的视频或任意 substring 替换。只有实际已知 CDN 的精确域名，才允许用同平台、HTTPS、具体作品 ID 的页面恢复原源；已有 `final_url` 中的具体原源优先于 Referer。其他平台、根目录、嵌套 URL、用户信息、自定义端口、HTTP 降级、冲突 Referer 均不用于恢复。

没有改下载网络策略、没有给失败响应强行放行、没有盲目扩充旧 `is_douyin_cdn` 失败兜底，也没有削弱 Range/长度/验证器/容器校验。

实际执行 `scripts/audit/media_source_probe.rs`，直接导入相同生产辅助函数及正式测试：**8 passed / 0 failed / 0 ignored**。覆盖默认根 Referer、别的视频、原任务首页不被另一个视频替换、合法 CDN 恢复、存储原源身份优先、跨平台/恶意域名与 query substring、无效来源和重复冲突头。相同测试随正式 Cargo 收录；独立脚本没有模拟整个 Manager。

## 修复后的真实 Manager 完整下载

根任务使用临时数据库、正式队列/worker 与显式 unknown 音轨选择，再次执行匿名公开样本。最终 **4/4 完整通过，exit 0，80.01 秒**；两条作品分别用 direct 和现有显式环境代理完成，不能用独立前缀请求代替：

| 样本与路径 | 实际整片字节 | 实际时长 | 整片 SHA-256 | 验证 |
| --- | ---: | ---: | --- | --- |
| 7694589931460594953，direct | 9921329 | 35.766667 秒 | abc97e15776b15cead6dbe5e456d4fb2279e35d8ad9bc120a5b2d9e67332260d | 实际音视频双轨、完整严格 decode |
| 7408129200156101907，direct | 16340971 | 60.666667 秒 | 25522797bee906aee4984f4c2b99ce7d73d9ad08c60c38b36b085f9d15fd3377 | 实际音视频双轨、完整严格 decode |
| 7694589931460594953，现有显式环境代理 | 9921329 | 35.766667 秒 | abc97e15776b15cead6dbe5e456d4fb2279e35d8ad9bc120a5b2d9e67332260d | 与 direct 相同整片 SHA，通过双轨与严格 decode |
| 7408129200156101907，现有显式环境代理 | 16340971 | 60.666667 秒 | 25522797bee906aee4984f4c2b99ce7d73d9ad08c60c38b36b085f9d15fd3377 | 与 direct 相同整片 SHA，通过双轨与严格 decode |

日志为 `TEMP/douyin-source-fixed.log`。这些是该两条公开样本在当时网络与服务状态下的实际完成证据，不代表所有作品、登录内容、直播、DRM 或长期可用性通过。

最终统一 Cargo round10 **1471 passed / 0 failed / 16 ignored，97.61 秒**，另 **9 项 Range 回归全通过，5.07 秒**。该轮已包含本 helper 的 8 项与根任务最新 endpoint 2 项；274 个 Rust 源文件快照对比 Changed=0。release round3 在此冻结生产源码上构建并 exit 0，随后以新起点和全新 TEMP 目录真实静态验收 **13/13 通过**。当前实际大小/SHA 和安装生命周期未验证边界见发布专项，未拿 round2 历史 SHA 代替。
