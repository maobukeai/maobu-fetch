export function selectBridgeMediaFormat(result) {
  const formats = Array.isArray(result?.formats) ? result.formats : [];
  const byHeight = (left, right) => Number(right.height || 0) - Number(left.height || 0);
  return formats.filter((item) => item.has_video && item.has_audio && !item.requires_ffmpeg).sort(byHeight)[0]
    || formats.filter((item) => item.has_video && !item.requires_ffmpeg).sort(byHeight)[0]
    || formats.find((item) => item.has_audio && !item.requires_ffmpeg)
    || formats[0];
}

export function bridgeMediaTask(result, pageTitle = "媒体下载") {
  if (result?.drm) throw new Error("检测到 DRM 保护，猫步下载器不会处理此内容");
  const format = selectBridgeMediaFormat(result);
  if (!format) throw new Error("没有找到可下载的媒体格式");
  const extension = format.extension || "mp4";
  const baseName = String(result.title || pageTitle || "媒体下载")
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .replace(/[. ]+$/g, "")
    .slice(0, 150) || "媒体下载";
  return {
    fileName: `${baseName}.${extension}`,
    media: {
      extractor: result.extractor,
      format_id: format.id,
      format_label: format.label,
      subtitles: [],
      thumbnail: result.thumbnail,
      requires_ffmpeg: Boolean(format.requires_ffmpeg),
    },
  };
}
