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
///   - "audio"：优先纯音频流（省空间/播客场景），无则回退最高清晰度。
/// 始终优先"音视频合一且无需 FFmpeg"的格式（避免依赖未安装的组件）。
export function selectBridgeMediaFormat(result, preference = "best") {
  const formats = Array.isArray(result?.formats) ? result.formats : [];
  const byHeightDesc = (left, right) => Number(right.height || 0) - Number(left.height || 0);
  const progressive = formats
    .filter((item) => item.has_video && item.has_audio && !item.requires_ffmpeg)
    .sort(byHeightDesc);
  const videoOnly = formats.filter((item) => item.has_video && !item.requires_ffmpeg).sort(byHeightDesc);
  const audioOnly = formats.filter((item) => item.has_audio && !item.has_video && !item.requires_ffmpeg);

  if (preference === "audio") {
    return audioOnly[0]
      || formats.find((item) => item.has_audio && !item.requires_ffmpeg)
      || progressive[0]
      || videoOnly[0]
      || formats[0];
  }
  const heightCap = preference === "1080" ? 1080 : preference === "720" ? 720 : 0;
  if (heightCap > 0) {
    const withinCap = progressive.filter((item) => Number(item.height || 0) <= heightCap);
    if (withinCap.length) return withinCap[0];
    // 用户上限高于现有任何清晰度或仅有超上限资源：回退默认策略，不空手而归。
  }
  return progressive[0]
    || videoOnly[0]
    || formats.find((item) => item.has_audio && !item.requires_ffmpeg)
    || formats[0];
}

export function bridgeMediaTask(result, pageTitle = "媒体下载", preference = "best", subtitlePreference = "all") {
  if (result?.drm) throw new Error("检测到 DRM 保护，猫步下载器不会处理此内容");
  const format = selectBridgeMediaFormat(result, preference);
  if (!format) throw new Error("没有找到可下载的媒体格式");
  const extension = format.extension || "mp4";
  const baseName = String(result.title || pageTitle || "媒体下载")
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .replace(/[. ]+$/g, "")
    .slice(0, 150) || "媒体下载";
  return {
    fileName: `${baseName}.${extension}`,
    format,
    media: {
      extractor: result.extractor,
      format_id: format.id,
      format_label: format.label,
      // 字幕语言按用户偏好过滤后透传给桌面端（P3-17 字幕接线 + 偏好）。
      subtitles: filterSubtitles(result.subtitles, subtitlePreference),
      thumbnail: result.thumbnail,
      requires_ffmpeg: Boolean(format.requires_ffmpeg),
    },
  };
}
