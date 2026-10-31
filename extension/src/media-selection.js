// 媒体任务构造：从 /v1/media/probe 结果挑选格式并生成任务载荷（纯函数）。

/// 按用户偏好过滤字幕语言列表。
///
/// probe 返回的 subtitles 是 yt-dlp 字幕键（语言代码，如 "zh-Hans"/"en"）。
///   - "none"：不带字幕；
///   - "zh"：优先中文字幕（没有任何中文时保留全部，避免意外丢失字幕）；
///   - 其余（默认 "all"）：保留全部（上限 20 条）。
export function filterSubtitles(subtitles, preference = "all") {
  const list = Array.isArray(subtitles) ? subtitles.slice(0, 20) : [];
  if (preference === "none") return [];
  if (preference === "zh") {
    const zh = list.filter((item) => /^zh/i.test(String(item || "")));
    return zh.length ? zh : list;
  }
  return list;
}

/// 按用户清晰度偏好挑选格式（P3-17）。
///
/// `preference`：
///   - "best"（默认）：最高清晰度，行为与历史版本一致；
///   - "1080" / "720"：不超过该高度的最高清晰度（无合适项时回退 best）；
///   - "audio"：明确选择音频，优先纯音频流，无则使用带音轨的格式。
/// FFmpeg 可用时优先最佳有声画质；旧桌面响应缺能力字段时保守选择单文件。
export function selectBridgeMediaFormat(result, preference = "best") {
  const formats = Array.isArray(result?.formats) ? result.formats : [];
  const originals = formats.filter((item) => item.original_file);
  if (originals.length) {
    if (String(result?.extractor || "").toLowerCase() !== "pikpak" || originals.length !== 1) throw new Error("请在桌面网盘文件选择器中明确选择原文件");
    return originals[0];
  }
  if (result?.media_type === "gallery") return formats.find((item) => item.image_url);
  if (result?.media_type === "collection") throw new Error("请在猫步下载器桌面端选择需要下载的分集");
  const ffmpegAvailable = result?.ffmpeg_available === true;
  const bitrate = (item) => typeof item.bitrate_kbps === "number"
    && Number.isFinite(item.bitrate_kbps) && item.bitrate_kbps > 0 ? item.bitrate_kbps : undefined;
  const compareBitrate = (left, right) => {
    const leftRate = bitrate(left), rightRate = bitrate(right);
    if (leftRate !== undefined && rightRate !== undefined) return rightRate - leftRate;
    return Number(rightRate !== undefined) - Number(leftRate !== undefined);
  };
  const quality = (left, right) => Number(right.height || 0) - Number(left.height || 0)
    || Number(right.width || 0) - Number(left.width || 0)
    || compareBitrate(left, right)
    || Number(Boolean(left.requires_ffmpeg)) - Number(Boolean(right.requires_ffmpeg))
    || Number(right.file_size || 0) - Number(left.file_size || 0);
  const allowed = formats.filter((item) => ffmpegAvailable || !item.requires_ffmpeg);
  if (preference === "audio" || result?.media_type === "audio") {
    const audio = allowed.filter((item) => item.has_audio && !item.audio_unknown).sort(quality);
    const selected = audio.find((item) => !item.has_video) || audio[0];
    if (selected) return selected;
    throw new Error("未找到可下载的音轨，请检查媒体权限与组件状态");
  }
  const audible = formats.filter((item) => item.has_video && item.has_audio && !item.audio_unknown);
  const candidates = audible.filter((item) => ffmpegAvailable || !item.requires_ffmpeg).sort(quality);
  const heightCap = preference === "1080" ? 1080 : preference === "720" ? 720 : 0;
  if (heightCap > 0) {
    const withinCap = candidates.filter((item) => Number(item.height || 0) <= heightCap);
    if (withinCap.length) return withinCap[0];
  }
  if (candidates.length) return candidates[0];
  if (audible.some((item) => item.requires_ffmpeg)) throw new Error("当前有声画质需要 FFmpeg 合并组件，请在桌面端设置中安装后重试");
  if (formats.some((item) => item.audio_unknown)) throw new Error("来源未确认音轨，未自动下载；请在桌面端手动选择‘音轨未确认’原文件，或更新组件后重新分析");
  throw new Error("未找到已确认带音轨的视频格式，请在桌面端检查并明确选择确定无声视频或纯音频");
}

export function bridgeMediaTask(result, pageTitle = "媒体下载", preference = "best", subtitlePreference = "all") {
  if (result?.drm) throw new Error("检测到 DRM 保护，猫步下载器不会处理此内容");
  const format = selectBridgeMediaFormat(result, preference);
  if (!format) throw new Error("没有找到可下载的媒体格式");
  const extension = format.extension || (format.original_file ? "" : "mp4");
  const baseName = String(result.title || pageTitle || "媒体下载")
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .replace(/[. ]+$/g, "")
    .slice(0, 150) || "媒体下载";
  return {
    fileName: format.original_file && (!extension || baseName.toLowerCase().endsWith(`.${extension.toLowerCase()}`)) ? baseName : `${baseName}.${extension}`,
    format,
    media: {
      is_live: result.is_live === true,
      extractor: result.extractor,
      format_id: format.id,
      format_label: format.label,
      // 字幕语言按用户偏好过滤后透传给桌面端（P3-17 字幕接线 + 偏好）。
      subtitles: filterSubtitles(result.subtitles, subtitlePreference),
      thumbnail: result.thumbnail,
      requires_ffmpeg: Boolean(format.requires_ffmpeg),
      url: format.url || undefined,
    },
  };
}
