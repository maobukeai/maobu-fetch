# M16：内置 fMP4 合并流式化与实际解码复验

日期：2026-10-31（Asia/Shanghai）。本记录覆盖 `media_muxer`，任务独立目录和 yt-dlp 结构化输出清单由媒体执行层另行记录。

## 修改与结果

- 保持 `merge_fragmented_mp4(video,audio,output)`，新增 `merge_fragmented_mp4_with_cancel(...,&CancellationToken)`。新增 `probe_fragmented_mp4_track_kind` 及可取消版本，按容器 `mdia/hdlr` 的 `vide/soun` 识别唯一轨道；不依靠文件名或大小。
- 新增 `validate_fragmented_mp4_with_cancel`，用于单轨或唯一视频加音频的双轨录制文件；支持同一 moof 的多个明确基址 traf，检查每个轨道、采样引用和末尾片段完整性，不改写输入。单轨识别入口仍拒绝双轨文件。
- 整文件读取及完整输出 Vec 已移除。顶级 box 仅扫描头部及范围；每输入 `moov` 最多 8 MiB、单个 `moof` 最多 4 MiB，`mdat` 使用 256 KiB 缓冲流式复制。元数据分配受明确上限约束并检查分配失败；没有与完整媒体长度相同的生产缓冲。没有增加依赖。
- 修复旧合并器重复生成两个 `mvex` 的问题，改为一个 `mvex`、两个 `trex`。输入可使用任意非零 track ID，统一重映射视频 1、音频 2，并同步 `tkhd/trex/tfhd/prft`；`mfhd` 全文件递增。验证并重定位绝对 `tfhd.base_data_offset`，保留相对片段偏移。
- 校验 box 32/64 位长度、唯一轨道、版本/标志、采样描述、解码时间、`trun` 采样字节范围必须落在完整 `mdat` 内；拒绝截断、错误引用、DRM 与外部数据引用。未定长度 `mdat` 在小于 4 GiB 时重写明确长度，避免追加音轨后把后续 box 吞入媒体数据。原位置索引 `sidx/mfra` 不复制到新文件，避免保留失效绝对偏移。
- 输出先写同目录唯一临时文件，逐段检查取消，flush/sync、准确长度核验后调用公共原子发布函数，禁止覆盖已存在的目标。失败或取消保留原始输入轨道；中间临时输出位于当前任务 staging。用户最终目标发布由媒体任务执行层处理。
- 原 900 余行文件拆为入口、box 读写、初始化元数据、片段校验/重定位、流式写出及测试模块。原有纯解析辅助回归搬至 test-only 模块；原有真实 Twitter opt-in 测试保留，没有删掉失败测试或新加 ignore 来跳过本批核心逻辑。

## 已实际执行

```powershell
powershell -NoProfile -File scripts/audit/run_muxer_probe.ps1
powershell -NoProfile -File scripts/audit/run_muxer_real_probe.ps1
```

生产源码独立回归：**19 passed / 0 failed / 1 ignored**，包含 8 个新增生产合并/验证行为用例、9 个原有本地回归及 2 个公共原子发布回归。唯一 ignored 是原有需要用户指定真实 Twitter 分离文件的手动测试，不是本批新增的跳过。

新增行为覆盖：误导文件名下正确识别视频/音频；任意 track ID 与一个 mvex；相对/绝对数据偏移；实际媒体 payload SHA-256 保持；错误 box、TS、普通 MP4、加密采样和错误 trun 拒绝；8 MiB 元数据上限；32/64 位 box 头；未定长度 mdat；已有目标保护；预先取消及 16 MiB 媒体复制进行中取消，不能发布半成品。

真实编解码复验使用电脑上已经安装的 FFmpeg/FFprobe，没有安装或下载工具。仅在随机临时目录生成 2 秒 96×64 测试图和 440 Hz 正弦波，分别编码 H.264/AAC。分别用相对 moof 基址、绝对 tfhd 基址两组 fMP4 输入调用同一生产合并器，两组均通过：

1. FFprobe 识别一条 H.264 视频和一条 AAC 音频；示例输出 45,714 字节，视频 2.000000 秒、音频 2.021333 秒。
2. 输入、输出各轨的每个 packet SHA-256、PTS、DTS、duration、size 完全一致。
3. FFmpeg `-xerror` 对合并后的两轨进行完整解码，退出码 0。
4. FFmpeg 额外生成相对/绝对基址的双轨、多 traf 同 moof 文件，两组均通过生产录制完整性验证器；本地用例确认末尾截断会明确失败，原文件不变。

这些证据支持当前支持范围内的容器重组、字节保持和可解码性；不能外推为所有平台和编码已经验收。

## 限制与剩余验收

- 内置流程只合并明确的一条清晰视频轨和一条清晰音频轨的 fMP4。TS 需要真正转封装，不能改后缀冒充 MP4；普通 progressive MP4、多轨、编辑列表/跨轨引用、扩展音频 SampleEntry 和未知关联 box 返回明确错误，媒体执行层可引导用户确认安装 FFmpeg。DRM 始终拒绝，不作为 FFmpeg 绕过入口。
- 超过 4 GiB 且自身声明 size=0 的 mdat 暂不支持；正常明确 64 位长度 mdat 使用流式复制。没有做多小时录制、数十 GiB 文件、磁盘满或突然断电实测；有界内存来自生产分配结构及上限回归，没有声称做过整机 RSS 压测。
- 多 traf 的片段若没有 absolute/default-base-is-moof 明确基址，录制验证入口会拒绝并要求正确转封装，不能把尚未支持的隐式跨轨寻址视为通过。结构验证不等同于包含所有编码器的完整码流解码器。
- 本批真实合成输入的 packet、时间戳与完整解码均通过；真实 Twitter/HLS 源、媒体任务独立目录、外部进程取消、最终用户路径重名策略和桌面提示仍需按各自入口验收。

格式校验依据：[W3C ISO BMFF Byte Stream Format](https://www.w3.org/TR/mse-byte-stream-format-isobmff/)、[FFmpeg fragmented MP4 选项](https://ffmpeg.org/ffmpeg-formats.html)。
