# 蓝奏 HTML 中间响应的文件完整性防护

日期：2026-10-31。范围仅限已确认的蓝奏来源文件类型与响应类型不符，不加入脚本执行、验证码绕过或通用网页下载限制。

## 已确认的缺口及证据边界

蓝奏 resolver 的最后一次 GET 原来只处理重定向；HTTP 200 的 HTML 响应仍可返回为“直链”。新建任务保持分享文件名及 `source="lanzou"`，用户未输入校验值时没有作者 SHA；分享列表中的大小也没有成为任务的预期长度。普通 HTTP 路径记录 Content-Type，并以该响应的长度及自身 SHA 完成传输与发布，不能据此证明内容就是用户所选文件。

因此，若某个中间页面以 identity 或未声明 Content-Encoding 的 HTTP 200 HTML 返回，旧路径允许把网页保存到所选二进制文件名并标为完成。此处由实际源码调用链确认，修复前没有执行新增本地夹具，不能将推导冒称实跑复现。

此前实网 Umi 中间页观测为 gzip HTML。普通 HTTP 已有非 identity 编码拒绝：若下载管理器仍收到 gzip 编码头，会在实际流响应规划处失败。因此不宣称该公开样本已发生“HTML 假完成”。浏览器自然得到最终地址后，Manager 下载 134,293,725 字节并匹配作者 SHA 的结果也不证明静态分享解析全链路通过；见[实网分层记录](2026-10-31-cloud-public-validation.md)。

## 最小修改

- `manager/cloud_content.rs` 仅针对 `source=lanzou`、文件名有明确二进制/压缩包/媒体/图片/二进制文档后缀的任务。成功响应的 MIME 为 `text/html` 或 `application/xhtml+xml` 时失败；大小写及参数正常处理。
- 错误正文只陈述响应被标为 HTML、与所选类型不符，可能为中间页或验证页；引导用户用浏览器正常打开分享并由扩展接管真正文件，不臆断需要登录。错误不含 URL、Cookie 或其他认证数据。
- 复用已有 `MEDIA_PROBE_ERROR:` 终止分支，避免将明确需要交互的响应当作普通瞬时网络错误反复重试。分片重试同样立即传播该错误，不包装丢失其终止语义。
- 防护检查需要覆盖初始 probe、普通流的实际响应及每个 Range 的实际响应。后两处避免 probe 正常后服务器改回 HTML。Range 会话只增加内存布尔值，不复制额外凭据。
- 普通流在打开/截断文件之前检查；Range 在读写 body 前检查，原分片仅以 append 打开、不截断。拒绝后的内容不进入完成发布，原有文件与续传数据保留。

明确下载 `.html` / `.htm` / `.xhtml`、未声明的扩展名、未知 MIME 及其他来源仍按既有 HTTP 行为处理。非 HTML MIME 未被当作真实性证明：服务器若把网页伪报为二进制，仍不在本次 MIME 防护范围内。此变更没有新增数据库或协议字段，不改变兼容标识、任务命令、签名或文件名。

## 验证设计与执行状态

**冻结源码的目标验证已实际通过：4项单元测试4/4、0失败（0.00秒）；无可见窗口的真实Manager聚合selector 1/1、0失败（0.49秒），内部执行下列10个场景。full13曾因栈溢出中止；测试入口heap-pin后的完整full14实际exit0：1494通过、0失败、16忽略（113.34秒），另Range 9通过、0失败（5.06秒）。默认聚合独立复跑亦实际1/1通过（109.91秒），不累加为新的全套测试项；release round5构建实际exit0，全新13项静态验收通过。失败历史不删除。**

根任务复用已编译并核对SHA的测试EXE `maobu-progress-backend-0b574a9b446a42d3bd70ef1cbb66b0bb/progress-tests-compile3.exe`，没有重新编译或更改冻结Rust源码。实际日志为 `%TEMP%/maobu-cloud-content-unit.log` 和 `%TEMP%/maobu-cloud-content-native.log`，已逐项读取测试名、失败数和耗时核对。原生聚合运行时显式设置 `MAOBU_FETCH_ISOLATED_TEST=1`、`MAOBU_TEST_NATIVE_ONLY=cloud-content`，并仅执行既有无窗口聚合测试的精确filter；没有运行真实GUI。

4 项单元测试覆盖：蓝奏明确文件类型及大小写/带参数 HTML MIME；明确 HTML、未知后缀与其他来源保留；未知/非 HTML MIME、非成功 HTTP 状态保持旧处理；Range 同规则及错误不泄露合成凭据。

新增 `production_http_tests/cloud_content_cases.rs` 使用独立 loopback 服务、临时 Store 和真实队列/worker，在现有单 Wry 无窗口聚合中执行以下 10 个场景：

1. 初始 HTTP 200 identity HTML 失败，无最终文件、无 Completed。
2. probe 正常、普通数据响应改为 HTML 时失败。
3. probe 正常、合法 Content-Range 的 206 分片改为 HTML 时失败。
4. overwrite 意图下已存在的用户目标文件保持原字节。
5. 实际请求从旧字节精确续传，HTML 拒绝后临时文件不被截断或追加。
6. 有效 gzip HTML、声明 octet-stream 的响应仍由既有 identity 编码校验拒绝。
7. 正常二进制完整下载。
8. 用户明确选择 HTML 文件完整下载。
9. 其他来源明确保存网页文件保持既有行为。
10. 未知 MIME 的蓝奏文件不被本规则全局拒绝。

负例不设置 expected_checksum，确保失败来自响应防护，而非夹具预先提供的哈希。正常例检查实际字节与长度。聚合复用已有无可见窗口的 Wry AppHandle，不开启原生悬浮窗，不注册系统关联、不读取个人数据库、不占用 17433。

## 完整组合回归的失败记录

`%TEMP%/maobu-final-cargo-round13.log` 实际记录：运行1508项测试期间，默认原生聚合在60秒后仍在执行，随后测试线程发生栈溢出，进程以 `0xc00000fd / STATUS_STACK_OVERFLOW` 中止。没有最终测试汇总，不能将此前已打印的进度点数当作完整通过计数。该日志没有无捕获的阶段定位，不能断言必由新增蓝奏场景引发，也没有证据据此宣布生产路径不存在栈问题。

根任务批准的最小组合处理仅改测试入口：把现有未分组的 `scenarios::run` future 放入 `Box::pin`，让 `runtime.block_on` 传递固定大小的指针；打印该未分组future的实际字节，并增加不含凭据的阶段开始/结束stderr。保留所有场景、断言、真实媒体/浏览器开关及原有线程/栈配置，没有调整 `RUST_MIN_STACK`。增加的 `future-size` selector 只测量并丢弃future，不执行场景，不计为行为验证通过。

复用明确新编的 `progress-tests-compile5.exe` 并先核对其SHA-256为 `6ceb2a45b02ab0ddba42ec448da15e07b8c7d15c2fab0d85a4589843d0e4cff1` 后，`future-size` 诊断实际exit0（0.01秒），日志 `%TEMP%/maobu-native-future-size.log` 明确输出 `ungrouped_bytes=26280`、`diagnostic_only=true`、`scenarios_executed=false`。测量的是Box内原未分组future的真实类型大小，不是Box指针大小。此结果只为大小诊断，不算新增一项行为通过。

26,280字节本体远小于常见2MB测试线程栈，不能据此证明“新增蓝奏场景导致巨型future超过线程栈”。实际poll调用链、构造/移动临时值和更深层future的栈占用仍未定位。入口heap-pin是单变量的栈复制压力缓解实验；生产下载语义没有因该诊断而改变。

随后根任务完整full14实际exit0，日志 `%TEMP%/maobu-final-cargo-round14.log`：1494通过、0失败、16忽略，113.34秒；另9项Range集成全通过，5.06秒。该轮使用 `--nocapture`，所有阶段开始/结束记录完整配对，包括蓝奏防护、HTTP、BT、混合预算、录制、真实媒体工具及Chrome/Edge桥，末尾到达 `done=aggregate`。没有增加线程栈或关闭场景。此结果证明上述冻结源码在本次完整组合下通过，不足以认定栈溢出的唯一根因，亦不宣称潜在生产栈问题已全部排除。

根任务随后再次核对compile5的SHA，以相同隔离、真实媒体及浏览器桥开关，使用exact filter独立复跑默认聚合，实际exit0、1/1通过、109.91秒；日志 `%TEMP%/maobu-native-aggregate-repeat.log` 同样记录所有阶段完成至 `done=aggregate`。没有调整 `RUST_MIN_STACK`。这次重复验证与full14分别留档，不把同一聚合再次运行算作新增不同测试；旧full13失败和根因未定边界保留。

## 未验证范围

release round5已实际构建成功（exit0），本轮构建起点UTC为 `2026-10-31T15:13:30.9942952Z`，全新TEMP静态验收13/13通过；机器可读证据为 `C:/Users/20269/AppData/Local/Temp/maobu-release-round5-bb4c91439d344c1d8f044df08133b8c1/release-inspection.json`。实际产物、大小、SHA及静态范围见[发布专项](2026-10-31-release-inspection.md)，不将构建/解包检查当作客户端或安装器已经运行。

本次没有重新抓取公网，不据此宣布蓝奏分享粘贴自动解析、浏览器真实接管或所有网盘完整文件通过。此前实网gzip HTML不能因本地identity负例而改写成已发生假完成。Windows原生交互与安装生命周期仍保留未验证状态。完整状态见[总整改报告](2026-10-31-remediation-report.md)，full13中止记录保留。
