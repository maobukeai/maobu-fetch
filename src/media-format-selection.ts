import type { MediaFormat, MediaProbeResult } from "./types";

export function selectOriginalFileFormat(result: MediaProbeResult): MediaFormat {
  const originals = result.formats.filter((format) => format.original_file);
  if (result.extractor?.toLowerCase() !== "pikpak" || originals.length !== 1) throw new Error("请在网盘文件选择器中明确选择需要的原文件");
  return originals[0];
}

const bitrate = (format: MediaFormat) => typeof format.bitrate_kbps === "number"
  && Number.isFinite(format.bitrate_kbps) && format.bitrate_kbps > 0 ? format.bitrate_kbps : undefined;
const compareBitrate = (left: MediaFormat, right: MediaFormat) => {
  const leftRate = bitrate(left), rightRate = bitrate(right);
  if (leftRate !== undefined && rightRate !== undefined) return rightRate - leftRate;
  return Number(rightRate !== undefined) - Number(leftRate !== undefined);
};
const quality = (left: MediaFormat, right: MediaFormat) => (right.height ?? 0) - (left.height ?? 0)
  || (right.width ?? 0) - (left.width ?? 0)
  || compareBitrate(left, right)
  || Number(left.requires_ffmpeg) - Number(right.requires_ffmpeg)
  || (right.file_size ?? 0) - (left.file_size ?? 0);

export function selectDefaultVideoFormat(formats: MediaFormat[], ffmpegAvailable: boolean): MediaFormat {
  const audible = formats.filter((format) => format.has_video && format.has_audio && !format.audio_unknown);
  const selected = audible.filter((format) => ffmpegAvailable || !format.requires_ffmpeg).sort(quality)[0];
  if (selected) return selected;
  if (audible.some((format) => format.requires_ffmpeg)) throw new Error("当前有声画质需要 FFmpeg 合并组件，请先安装后重试");
  if (formats.some((format) => format.audio_unknown)) throw new Error("来源未确认音轨，不能自动当作有声视频；可手动选择‘音轨未确认’原文件，或更新媒体组件后重新分析");
  throw new Error("未找到已确认带音轨的视频格式；如需确定无声的视频或纯音频，请明确选择对应格式");
}

export function formatTrackDescription(format: MediaFormat): string {
  if (format.original_file) return "";
  if (format.audio_unknown) return " · 音轨未确认（可手动下载原文件）";
  if (format.has_video && format.has_audio && !format.requires_ffmpeg) return " · 轻量单文件";
  if (format.has_video && !format.has_audio) return " · 仅视频（无声）";
  if (!format.has_video && format.has_audio) return " · 仅音频";
  return "";
}

/** 合集逐集解析时应用同一画质上限；缺少组件时只允许有声单文件。 */
export function collectionFormatSelector(preference: string, ffmpegAvailable: boolean): string {
  const cap = preference === "720p" ? "[height<=720]" : preference === "480p" ? "[height<=480]" : "";
  const progressive = `best${cap}[vcodec!=none][acodec!=none]`;
  return ffmpegAvailable ? `bestvideo*${cap}+bestaudio/${progressive}` : progressive;
}
