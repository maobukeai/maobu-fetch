# M05：暂停与停止保存录制的完整交互契约

日期：2026-10-31。范围限定为录制任务的识别、任务行控制、停止授权、持久化状态和实际发布，不新增录制服务或站点能力。

## 确认的问题与修改

原 manager 把普通 `pause` 同时当成授予 `finalize_live` 的操作；界面只显示“暂停”，用户点击后可能把直播录制合并发布为 Completed。这与“暂停保留恢复数据”和明确停止保存的区别不一致。

- 普通 `pause`、取消、删除和退出均不授予保存许可。暂停停止实际活动连接，保留分片/恢复清单，任务保持 Paused。
- 明确停止时若首个片段尚无可保存数据，等待 worker 退出后仅把仍为 Downloading 的记录原子改为 Paused、清零活动连接并保留可操作原因；不覆盖已完成、失败、取消或删除记录。真实首段无数据负向与原流程一并实跑通过，避免留下没有 worker 的 Downloading 孤儿。
- 在已有 `task_action` 命令上新增 action 值 `finish-recording`。只有此明确操作才授予许可；等待真实 worker、容器校验和原子发布完成后，才能返回保存成功。失败保留原始数据并返回中文原因。
- 任务行对当前确认正在录制的任务显示独立“停止并保存录制”方形按钮；原 Pause 继续表示暂停。Stop 有明确 title/aria-label、焦点、忙碌禁用与错误反馈，不凭颜色区分。暂停后可先恢复录制，再选择停止保存。
- `MediaSelection` 与 `MediaProbeResult` 增加安全默认 `is_live=false`，TypeScript 为兼容可选字段；使用既有 `media_json`，无 SQLite 列变更，不迁移或重写旧已完成任务。
- HLS 标记来自实际解析的无 ENDLIST Media Playlist；URL 后缀、未知时长和 Master URL 本身不构成直播证明。工具探测和 before_dl 标记使用真实 `is_live/live_status`，历史 `was_live` 或 upcoming 不冒充正在直播。
- 每代 WorkerControl 的 `observed_live` 默认 false。启动会清掉旧/客户端直播提示；只有该代生产解析或实际 before_dl 观测才能授权 Stop。单独伪造请求里的 `is_live=true` 不能把普通文件前缀保存为完成录制。
- 直播观察通过原子 Store helper 只更新已有 Downloading HTTP 任务的媒体字段，保留用户选项和进度；暂停、已完成、取消、删除之后的晚到观察不能改写或复活记录。
- 原生 HLS 使用明确 `native-hls` 路由标记。暂停后恢复仍走原生 HLS，不因 media 从 None 变为 Some 而误走 yt-dlp，不要求为普通 HLS 安装工具；真正需要独立音视频合并的选择仍保留工具路径。

相关实现分为 `manager/live_recording.rs`、`store/media_live.rs`、原有 HLS adapter、`FinishRecordingButton.tsx` 与小型可见性 helper；媒体探测/工具元数据和 HLS engine 的真实标记由各自模块维护。

## 实际后端验证

| 验证 | 实际结果 |
| --- | --- |
| `store::media_live` 最终过滤 | **3 passed / 0 failed / 0 ignored**，0.04 秒。旧 MediaSelection/Probe JSON 缺字段默认 false；原子直播字段更新保留选项/字节；失败收尾只把 Downloading 改为 Paused，不覆盖暂停/完成/失败/取消，不复活删除。 |
| 同一真实 Wry 上下文的 `recording-flow` 最终过滤 | **1 passed / 0 failed / 0 ignored**，约 **6.20 秒**。实际 Manager、SQLite、loopback 服务、真实 TS、HLS、停止与发布都参与；包含真实首段无数据负向，没有把 mock UI 当作后端验收。 |
| TypeScript 可见性 | **8 个断言通过**：实际 live/Downloading 才可停止；普通 m3u8、未知长度、显式 false、暂停/完成/失败/取消、BT 均不显示 Stop。 |
| `npx tsc --noEmit` | 退出 0。 |

原生场景以已存在的 FFmpeg 仅作为测试生成器，生成真实约 2 秒 H.264/AAC MPEG-TS，源文件含音视频。应用的隔离运行配置指向不存在的 yt-dlp/FFmpeg/FFprobe 路径，并实际断言 resolver 不可用；没有修改 PATH 或用户设置，没有安装组件。测试源 `/live` 没有 `.m3u8` 后缀，服务实际返回无 ENDLIST 的播放列表。

场景逐步验证：

1. 开始后等待实际直播观测与真实完整 TS 切片；普通 pause 后为 Paused、活动连接停止、没有最终文件。恢复目录中的真实切片长度/SHA 与源匹配，继续等待后全部指纹不变。
2. resume 后仍为 native-hls，工具需求为 false；只有明确 `finish-recording` 才完成，最终后缀是实际 `.ts`，字节逐一等于源，并由测试 oracle FFmpeg 对视频和音频完整解码。
3. 新录制分别执行 cancel、仅删记录、shutdown，保存许可均为 false、没有最终录制文件，恢复分片保留。
4. 对普通有限 HTTP 文件伪造 `media.is_live=true`，在真实活动下载期间 Stop 明确拒绝，任务仍下载；随后普通暂停没有最终文件。
5. 配置隔离的实际 FFmpeg/FFprobe 后再录制，明确停止得到真实 `.mp4`，双轨完整解码通过；ENDLIST 的有限 HLS 正常完成但不标直播，之后 Stop 拒绝且已完成记录原值不变。
6. `/waiting` 返回实际无 ENDLIST 播放列表，`/silent.ts` 返回真实片段的 Content-Length 但始终不发送首个 body 字节。测试确认真实首段 GET 已到达、实际 downloaded_bytes 为 0 后再执行 Stop：返回“恢复录制”原因，记录为 Paused、活动连接为 0、worker 句柄已移除，没有最终文件，没有假报保存成功。

实际结果标记：`pause_preserved=true`、`resumed_without_tools=true`、`explicit_finish_completed=true`、`silent_first_segment_paused=true`、`cancel_delete_exit_not_published=true`、`forged_vod_rejected=true`、`tool_decode=true`。

最终日志为 `%TEMP%/maobu-recording-final-model-tests.log`、`%TEMP%/maobu-recording-final-manager-tests.log`。前轮正向日志为 `%TEMP%/maobu-recording-model-tests.log`、`%TEMP%/maobu-recording-manager-tests.log`（2 项、5.14 秒，未含新增无数据负向），不混用结果时间层级。可复现的单 Wry 入口使用 `MAOBU_TEST_EXTERNAL_MEDIA=1`、`MAOBU_TEST_NATIVE_ONLY=recording-flow`；显式工具测试前提不满足会失败，未开启工具 opt-in 的普通测试不会下载或安装工具。

## UI 与协议边界

真实 TaskRow 最终已完成 **19 组 / 14 张截图**的浏览器验证，证据为 `%TEMP%/maobu-recording-ui-Nzfmx4/results.json`，日志 `%TEMP%/maobu-final-recording-ui.log`。覆盖 1280×720、1440×900 的浅深色与 100%/125%，780×540 压力窗口及英文 125%。标签单行省略且 title/aria 保留全文；Stop 与真实连接文本间距至少 2px，英文两窗实测 3.75px。键盘分别触发 Pause 与 Stop，忙碌禁用、防重复发送及失败不假报成功均通过；未知总长的65536个实际录制字节显示64KB，不伪造0%。此 UI 结果与后端真实下载测试分别记录，不相互替代。

桌面原有命令名和请求结构保持兼容，`finish-recording` 是明确桌面动作；旧扩展的 pause 继续表示暂停，不会悄悄获得保存录制许可。媒体响应追加字段为可选，旧请求和旧 JSON 保持可读。

HLS engine 的真实标记及外置 AUDIO 组拒绝由[HLS 专项](2026-10-31-m3u8-fixes.md)记录；工具 metadata/凭据/进程退出由[媒体执行专项](2026-10-31-media-execution-fixes.md)记录。真实 UI 证据及项目最终全套数字由主任务汇总，本文不把专项通过写成全项目已发布完成。

没有验证真实站点长期直播、多小时断网恢复、磁盘满、掉电或系统 DPI。无工具 TS 的结构验证不等于通用解码器；本次测试额外使用 oracle 完整解码，只证明这个真实样本。DRM 仍明确拒绝，没有绕过或降低容器完整性校验。
