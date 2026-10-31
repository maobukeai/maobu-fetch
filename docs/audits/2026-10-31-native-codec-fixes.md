# 原生媒体编码与音轨真实性补充复验

日期：2026-10-31（Asia/Shanghai）。这是公网验收新带出的生产缺口及必要修复，不能用此前 release round2 的静态验收或模拟媒体数据掩盖。当前生产源码已冻结，统一全测与 release round3 结果由总台账续记。

后续最终统一 Cargo round10 已实际通过：**1471 passed / 0 failed / 16 ignored，97.61 秒**，另 **9 项 Range 全通过，5.07 秒**。本模块 18 项及补入原 SSR 测试的双结构 URL 绑定负向均在该轮重新执行；历史定向结果保留。两条抖音样本经过必要 source 修复后，真实 Manager 在 direct/现有代理各完整下载，最终 4/4 双轨与严格 decode 通过，见 `2026-10-31-douyin-source-fixes.md`，不将该证据泛化到全部平台。

## 已证实的问题

- 抖音视频 converter 无条件写 `h264/aac`，而 SSR 规范化只保留尺寸、时长、URL，既没有保存实际 codec，也没有音轨证明。
- 抖音直播 FLV/HLS converter 同样固定 `h264/aac`，并固定 `tbr=2560`；当前直播抓取仅提供 URL、状态与标题，不能证明这些编码或码率。
- Twitter native MP4 variant 只有 MIME、码率和 URL 时也被无条件写成 `h264/aac`。已有 optional yt-dlp 探测又以 `has_audio` 为唯一采纳条件：此前假 AAC 使它不触发；去掉假 AAC 后，native 空列表时工具的合法 unknown 候选仍会被丢弃。

## 实际修改

`media_platforms/native_tracks.rs` 只读取绑定当前媒体的明确 `vcodec/acodec`、`video_codec/audio_codec`、`videoCodec/audioCodec` 字符串，保留实际 codec 标识和明确的 `none`。缺失、null、unknown、无效类型保持未知；不从 MP4/FLV、URL、尺寸、`is_h265=false`、`has_audio`、音量或独立 music/BGM 数据推导 AAC。

抖音 SSR 保留这些明确字段。若 playAddr 包含多个候选，只把 URL 排序后实际选中项的 codec 绑定到当前 play_addr；不会把另一编码或另一音轨状态复制过来。Twitter 只采用当前 variant 的明确 codec，避免拿 video_info 的总级信息给所有 variant 宣称音轨。直播的固定码率也移除，没有元数据时保持未知。

`media/native_probe_fallback.rs` 集中 optional 工具选择规则：native 无候选时可接受有合法 HTTP(S) 地址的工具 unknown 候选，但其 `audio_unknown=true/has_audio=false` 保持；native 已有 unknown 时优先工具明确音轨，空、坏或普通工具失败不抹掉已有候选；双方都无候选返回可操作错误。明确的 DRM JSON、工具 DRM 错误和取消不能被 optional fallback 吞掉。纯图片或已确认音轨不再触发这次额外探测。

没有新增公开模型字段或依赖，没有新增联网挑战处理，没有将未知当无声。未知格式仍可由用户明确选择；默认自动视频选择继续拒绝未知音轨。实际下载后的 FFprobe/容器检查沿用既有生产路径，不会因 converter 宣称 AAC 而跳过。

## 已执行的专项回归

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib native_tracks
cargo test --manifest-path src-tauri/Cargo.toml --lib native_probe_fallback
```

实际结果：**10/10 + 8/8，通过 18 项，0 failed、0 ignored**。第一组正常重编生产测试 EXE，构建耗时 22.78 秒；第二组使用同一构建，未修改 EXE 或删除失败用例。

10 项 codec 回归包含 4 项明确字段/未知值/SSR 绑定辅助解析，以及 6 项真实 converter→正式格式 parser→默认选择链路。分别核验合法明确 codec 原样保留、`none` 为已知静音且不自动当作有声、unknown 保留可手选 URL 但拒绝自动选择、SSR 实际选中 variant、Twitter 高画质未知不得挤掉低画质已确认音轨，以及直播不再伪造 codec/码率。

8 项 fallback 回归覆盖 native 空/工具 unknown、native unknown/工具 confirmed、空/无效 URL/无 URL/损坏/失败工具结果保留原候选、双方 unknown 保留原候选、双方空明确失败、根/格式/requested_formats 的 DRM 与 DRM 错误、纯图片/确认音轨不额外探测，以及取消不转成功。

日志为 `TEMP/maobu-native-tracks-regression.log`、`TEMP/maobu-native-probe-fallback-regression.log`。上述用例使用合成元数据验证正式生产解析与选择函数，不代表这些平台已经完成新的公网下载。总任务另行运行匿名公开样本、真实 Manager worker、实际 FFprobe 与完整 decode，必须按其实际结果记录，不能用此 18 项推断。

## 构建边界

release round2 的 13/13 静态检查和实际 SHA 仍是该轮真实历史证据，但它未包含本次 codec/fallback 与同期 PikPak 修复，不能当作最新源码产物。后续 root 已完成 round3，新的 UTC 起点 `2026-10-31T08:20:23.6608125Z`、全新 TEMP 目录 fresh 静态验收 **13/13 通过**；当前最终实际 SHA 与仍未验证的安装生命周期见 `2026-10-31-release-inspection.md`，该报告保留 round2 历史表而不冒充最新。
