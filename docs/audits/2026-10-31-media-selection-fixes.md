# 媒体默认格式与 B 站选集修复记录

日期：2026-10-31。对应原审查 M06、M07、M13。此文档记录本轮修改和实际证据，不把组件替身测试视为公网平台完整下载验收。

## M06：默认选择有声画质

- 后端新增 `media/format_selection.rs`，桌面新建任务新增 `src/media-format-selection.ts`，扩展同步 `media-selection.js`。
- 默认视频必须同时具有视频与已确认音轨；新增 `audio_unknown=false` 安全默认字段，实际 codec 为 null/空/unknown/N/A 时保持未确认，尺寸、格式ID不能证明有声。未知原文件仍能明确手选，UI标注“音轨未确认”，不误标“无声”；扩展不自动下载未知音轨。
- 明确 `vcodec=none` 也不因封面尺寸而当成视频。native Twitter 可在已经安装 yt-dlp 时同会话有界补探测 manifest codec，仍未知则保留手选，不自动装工具。
- 默认视频必须同时具有视频与音轨。4K 纯视频不会覆盖 360P 有声候选；纯音频只在音频页面、音频偏好或用户明确选择时使用。
- FFmpeg 可用时比较有声候选，1080P 合并可优先于 360P 单文件；同分辨率先比较真实已知码率层，同码率优先无需合并的文件。没有组件时只自动选直接有声文件，只有合并候选则明确提示安装组件。
- 响应格式追加可选 `bitrate_kbps`：只取实际有限正数 `tbr`，或按已存在的音视频轨完整组合 `vbr/abr`；两路合并只有两路码率都已知时相加。缺失保持未知，不伪造为 0 kbps；相同分辨率中优先可明确比较的已知码率，均未知时单文件优先。
- 合并候选仅由已确认的分离视频与音轨共同产生，并使用确切 video.id+audio.id，防止泛 best 另选与展示属性不同的轨道，保留真实最高视频尺寸；大小仅在两路都已知且求和不溢出时显示。没有音轨不会构造假有声格式。
- 新建任务再次探测前清空旧格式，探测失败不会沿用旧 ID 下载同 ID 的无声替代流。无有效选择时拒绝创建。下拉列表明确标注“仅视频（无声）”“仅音频”。
- 探测响应追加 `ffmpeg_available`，由 `resolve_ffmpeg` 实际结果赋值；旧 JSON 默认 false，旧桌面缺字段时扩展保守选择单文件。桌面实时组件状态可更新探测时的旧状态。
- 自动媒体任务由主 agent 接入同一 Rust helper；音频页面单独选真实音轨。
- 追加 probe DRM 修复：`media/drm.rs` 同时拒绝根对象、`formats`、`requested_formats` 明确声明的 `_has_drm`/`has_drm`，false 字段不能覆盖另一个 true；分集复用同一检查。仅检查已声明信息，不声称已检测未知加密，也不尝试绕过。
- 用户 B 站真实样本另外发现 `filesize:null` 阻断估计值回退，现先取合法精确大小，再取有效估计值；`file_size_approximate` 缺失默认 false，估计与合并计算量在列表明确显示“约”。不把估计长度作为完整性判断，实际完整下载结果见 `2026-10-31-bilibili-user-sample.md`。

## M07：B 站选集与画质接线

- 保留 `media::probe` 默认只探测当前视频的语义；新增内部 `probe_collection`。桌面“分析媒体”显式请求选集，扩展与 HTTP 自动任务的请求保持原样。
- 显式 `?p=N` 仍是单集。桌面合集模式在 B 站使用 `--yes-playlist --flat-playlist`，依赖 yt-dlp 返回真实分集链接；不递归预取所有分集流。
- `media/collection.rs` 把 playlist 的 entries 转成 `Collection + MediaEpisode`，包含真实链接、编号、可用标题和时长；不捏造大小或画质。空/无效/重复编号或链接、DRM、非 B 站分集链接均明确拒绝。
- 有界读取最多 1001 项；超过 1000 集明确提示拆分，避免静默截断。访问受限或 null 分集不被悄悄跳过。
- 画质偏好传递为逐集格式选择表达式，720P/480P 上限实际生效；有 FFmpeg 时允许合并，缺少时只选有声单文件。每一集创建请求保留其真实 `?p=N` 或节目地址。
- 选集支持 Space/Enter 切换并提供 checkbox/aria-checked 语义。画质文案改为“最佳可用画质”，不承诺未取得的 1080P/4K；缺组件说明保持可见。

## M13：PikPak 原文件属性

- 不再固定标注1080P/4K、1920×1080或有音视频；保留真实文件名、大小、MIME/扩展名，未知尺寸/码率/音轨为待检测。普通非媒体文件或多文件分享提示使用既有网盘文件选择器，不伪装视频。
- 追加安全默认 `original_file=false`，Rust/TS/扩展仅对PikPak且唯一原文件候选走独立选择，不放宽视频默认必须有声的约束。原文件名含真实扩展名时保留，音频不被强行补`.mp4`。
- 实际生产helper fixture验证视频/音频/图片/普通压缩包、未知属性与大小；真实Edge新增原文件提交场景，确认显示待检测并保留原始文件名。没有进行真实PikPak账号分享下载验收。

## 兼容性

- `/v1` 版本、HMAC 算法、X-Luma-* 头、端口均未更改。能力字段为响应追加；Rust 反序列化有安全默认值，TS 字段可选。后续直播交互在已有 media 对象追加可选 is_live，缺失默认 false；服务端不信任请求标记作保存录制授权。
- Tauri 既有 `media_probe` 命令追加可选 `include_collection`，默认 false；前端使用 Tauri 参数 `includeCollection`。旧前端不传时保持单视频。
- 本记录仅描述选择与属性修改；媒体执行随后另行修复，见 `2026-10-31-media-execution-fixes.md`。工具安装和用户已完成文件未改动。

## 实际验证

| 验证 | 结果与证据 |
| --- | --- |
| TypeScript 编译 | `npx tsc --noEmit` 通过。 |
| 默认格式与合集表达式 | `npx tsx src/media-format-selection.test.ts`：17 项选择检查通过；已由主 agent 接入根 check。 |
| 扩展模拟 Chrome API 与协议回归 | `npm --prefix extension run check`：193/193 通过，包含新能力字段缺失、合并/单文件、真实码率优先、拒绝无声自动选择、显式音频、图集、503令牌保留和直播标记兼容场景。日志 `%TEMP%/maobu-extension-media-check.log`。 |
| Rust 媒体回归 | Windows CommonControls 测试 Manifest 已由根修复；当前已构建 EXE `media::` 过滤实际 **67 通过 / 0 失败 / 1 内部 child fixture 忽略**，包含格式/合集/原文件/DRM、未知音轨、源尺寸不能覆盖明确无视频、字幕与主恢复journal等生产helper。日志 `%TEMP%/maobu-media-execution-tests.log`。 |
| 真实 Edge 组件回归 | `node scripts/audit/media-ui.mjs` 通过：实际 NewTaskDialog 在有/无 FFmpeg 下提交正确格式，失败二次探测不提交无声流；键盘取消 P1 后只提交 P2，480P 表达式实际携带。另实测未知音轨阻止自动提交、键盘显式选择保留手选，合计6组；全部 API 使用合成替身，没有连接真实桌面。 |
| 视觉 | 1280×720、1440×900，100%/125% CSS 与设备缩放，浅/深共 8 张截图；检查分集与确认按钮在视口内，已目检。最新证据 `%TEMP%/maobu-media-ui-V2DvVe/results.json`，另含大小“约”断言。不是改变 Windows 系统 DPI。 |
| 公网 B 站多 P 探测 | 对上游 yt-dlp 测试中的公开多 P 地址执行真实有界 flat playlist 探测，返回 HTTP 412；没有关闭验证或尝试绕过。证据 `%TEMP%/maobu-bilibili-collection-live.json`。 |

本轮新增 Rust 测试执行生产 helper，而非复制实现：有声默认、组件能力、高分辨率无声排除、同画质单文件偏好、真实码率解析与排序、真实合并尺寸与大小、合集转换、无效/DRM/重复/截断拒绝、桌面选集意图与显式单 P 命令参数、根/格式/已选轨道 DRM 声明拒绝。

## 验证边界

真实账号的会员/地区限制、多 P 与番剧完整下载、真实浏览器→桌面端媒体传输仍需隔离用户数据下验收；本地真实 FFmpeg/内置合并双轨与完整解码已在执行层记录通过。当前 HTTP 412 不能算适配通过。普通 UGC 单视频不会自动扩大为整个 UP 主合集；支持 yt-dlp 能识别并返回 playlist 的明确合集地址和多 P 地址。
