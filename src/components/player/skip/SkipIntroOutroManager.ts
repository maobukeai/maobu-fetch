import type {
  ChapterMatch,
  ResolvedSkipSettings,
  SeriesSkipSetting,
  SkipIntroOutroConfig,
} from "./SkipTypes";

export const SKIP_CONFIG_STORAGE_KEY = "maobu_player_skip_intro_outro_config";

/** 默认跳过配置：动漫与电视剧的黄金 OP/ED 长度普遍为 90 秒 (01:30) */
export const DEFAULT_SKIP_CONFIG: SkipIntroOutroConfig = {
  enabled: true,
  autoDetect: true,
  defaultIntroSeconds: 90,
  defaultOutroSeconds: 90,
  autoPlayNextOnOutro: true,
  seriesSettings: {},
};

/** 从 localStorage 读取跳过配置 */
export function loadSkipConfig(): SkipIntroOutroConfig {
  try {
    const raw = localStorage.getItem(SKIP_CONFIG_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_SKIP_CONFIG };
    const parsed = JSON.parse(raw) as Partial<SkipIntroOutroConfig>;
    return {
      enabled: typeof parsed.enabled === "boolean" ? parsed.enabled : DEFAULT_SKIP_CONFIG.enabled,
      autoDetect: typeof parsed.autoDetect === "boolean" ? parsed.autoDetect : DEFAULT_SKIP_CONFIG.autoDetect,
      defaultIntroSeconds:
        typeof parsed.defaultIntroSeconds === "number" && parsed.defaultIntroSeconds >= 0
          ? parsed.defaultIntroSeconds
          : DEFAULT_SKIP_CONFIG.defaultIntroSeconds,
      defaultOutroSeconds:
        typeof parsed.defaultOutroSeconds === "number" && parsed.defaultOutroSeconds >= 0
          ? parsed.defaultOutroSeconds
          : DEFAULT_SKIP_CONFIG.defaultOutroSeconds,
      autoPlayNextOnOutro:
        typeof parsed.autoPlayNextOnOutro === "boolean"
          ? parsed.autoPlayNextOnOutro
          : DEFAULT_SKIP_CONFIG.autoPlayNextOnOutro,
      seriesSettings: parsed.seriesSettings && typeof parsed.seriesSettings === "object"
        ? parsed.seriesSettings
        : {},
    };
  } catch {
    return { ...DEFAULT_SKIP_CONFIG };
  }
}

/** 持久化保存跳过配置 */
export function saveSkipConfig(config: SkipIntroOutroConfig): void {
  try {
    localStorage.setItem(SKIP_CONFIG_STORAGE_KEY, JSON.stringify(config));
  } catch (err) {
    console.error("Failed to save skip intro outro config to localStorage:", err);
  }
}

/**
 * 从文件路径与文件名推导剧集归属 Key（Series Key）。
 * 优先采用所在父文件夹名称，次优剥离文件名中的集数后缀。
 */
export function getSeriesKey(filePath: string, fileName?: string): string {
  if (!filePath) return "default_series";

  // 规范化路径分隔符为正斜杠
  const normalized = filePath.replace(/\\/g, "/");
  const parts = normalized.split("/").filter(Boolean);

  // 如果有父目录，且父目录不是驱动器根目录（如 C: 或 /）
  if (parts.length >= 2) {
    const parent = parts[parts.length - 2];
    if (parent && !parent.endsWith(":") && parent !== "Downloads" && parent !== "下载") {
      return `folder:${parent.trim()}`;
    }
  }

  // 若无可用父目录，从文件名提取剧集公共名
  const name = fileName || parts[parts.length - 1] || "";
  const cleaned = name
    .replace(/\.[a-zA-Z0-9]+$/, "") // 去掉扩展名
    .replace(/(\[.*?\]|\(.*?\))/g, "") // 去掉中括号小括号压制组信息
    .replace(/(?:S\d+E\d+|EP\d+|第\d+集|\bE\d+\b|\b\d{1,3}\b)/i, "") // 去除集数标记
    .trim();

  return cleaned ? `series:${cleaned}` : "default_series";
}

/**
 * 判断是否处于剧集场景（看剧模式）：
 * 1. 同目录播放列表有多个视频；
 * 2. 文件名包含剧集编号模式（如 S01E02、EP03、第04集、05 等）。
 */
export function isEpisodeLike(fileName: string, playlistCount = 0): boolean {
  if (playlistCount > 1) return true;
  if (!fileName) return false;

  const episodePatterns = [
    /S\d+E\d+/i,
    /EP\s*\d+/i,
    /第\s*\d+\s*[集话回]/,
    /[\[\(]\s*\d{1,3}\s*[\]\)]/,
    /[_\-\s]\d{1,3}[_\-\s\.]/,
  ];

  return episodePatterns.some((pattern) => pattern.test(fileName));
}

/** 片头章节关键词匹配 */
const INTRO_REGEX = /(?:^|\b|_|-)(?:op|opening|intro|片头|序幕|前奏|导视|序章)(?:$|\b|_|-)/i;
/** 片尾章节关键词匹配 */
const OUTRO_REGEX = /(?:^|\b|_|-)(?:ed|ending|outro|片尾|演职员|尾声|下集预告|预告|credits|staff)(?:$|\b|_|-)/i;

/**
 * 从 HTMLVideoElement 内嵌的 textTracks 解析章节信息
 */
export function detectChaptersFromVideo(video: HTMLVideoElement | null): ChapterMatch | null {
  if (!video || !video.textTracks || video.textTracks.length === 0) {
    return null;
  }

  const duration = video.duration;
  if (!duration || isNaN(duration) || duration <= 0) return null;

  for (let i = 0; i < video.textTracks.length; i++) {
    const track = video.textTracks[i];
    if (track.kind === "chapters" && track.cues && track.cues.length > 0) {
      let introSeconds: number | undefined;
      let introTitle: string | undefined;
      let outroSeconds: number | undefined;
      let outroTitle: string | undefined;

      for (let j = 0; j < track.cues.length; j++) {
        const cue = track.cues[j] as VTTCue;
        const text = cue.text || "";

        // 识别片头（位于前 25% 且标题匹配）
        if (cue.startTime < duration * 0.25 && INTRO_REGEX.test(text)) {
          introSeconds = Math.round(cue.endTime);
          introTitle = text;
        }

        // 识别片尾（位于后 40% 且标题匹配）
        if (cue.startTime > duration * 0.6 && OUTRO_REGEX.test(text)) {
          outroSeconds = Math.round(duration - cue.startTime);
          outroTitle = text;
        }
      }

      if (introSeconds !== undefined || outroSeconds !== undefined) {
        return { introSeconds, outroSeconds, introTitle, outroTitle };
      }
    }
  }

  return null;
}

/**
 * 智能决策当前视频生效的片头片尾时长
 */
export function resolveSkipSettings(
  filePath: string,
  fileName: string,
  config: SkipIntroOutroConfig,
  playlistCount = 0,
  chapterMatch?: ChapterMatch | null
): ResolvedSkipSettings {
  const isEpisode = isEpisodeLike(fileName, playlistCount);
  const seriesKey = getSeriesKey(filePath, fileName);

  if (!config.enabled) {
    return {
      introSeconds: 0,
      outroSeconds: 0,
      enabled: false,
      autoDetect: config.autoDetect,
      autoPlayNextOnOutro: false,
      source: "disabled",
      sourceLabel: "已关闭跳过",
      seriesKey,
      isEpisode,
    };
  }

  // 1. 优先：视频内置章节匹配识别（最精准）
  if (config.autoDetect && chapterMatch && (chapterMatch.introSeconds || chapterMatch.outroSeconds)) {
    return {
      introSeconds: chapterMatch.introSeconds ?? config.defaultIntroSeconds,
      outroSeconds: chapterMatch.outroSeconds ?? config.defaultOutroSeconds,
      enabled: true,
      autoDetect: true,
      autoPlayNextOnOutro: config.autoPlayNextOnOutro,
      source: "chapter",
      sourceLabel: `已自动识别章节 (${[
        chapterMatch.introTitle ? `片头:${chapterMatch.introTitle}` : "",
        chapterMatch.outroTitle ? `片尾:${chapterMatch.outroTitle}` : "",
      ]
        .filter(Boolean)
        .join(" / ")})`,
      seriesKey,
      isEpisode,
    };
  }

  // 2. 次优：专属剧集/系列记忆配置
  const seriesSetting = config.seriesSettings[seriesKey];
  if (seriesSetting) {
    return {
      introSeconds: seriesSetting.introSeconds,
      outroSeconds: seriesSetting.outroSeconds,
      enabled: true,
      autoDetect: config.autoDetect,
      autoPlayNextOnOutro: config.autoPlayNextOnOutro,
      source: "series",
      sourceLabel: `已匹配本剧集专属记忆 (${seriesKey.replace(/^(folder|series):/, "")})`,
      seriesKey,
      isEpisode,
    };
  }

  // 3. 兜底：剧集智能识别默认预设或全局默认值
  return {
    introSeconds: config.defaultIntroSeconds,
    outroSeconds: config.defaultOutroSeconds,
    enabled: true,
    autoDetect: config.autoDetect,
    autoPlayNextOnOutro: config.autoPlayNextOnOutro,
    source: "default",
    sourceLabel: isEpisode ? "看剧模式（默认智能预设）" : "全局默认设置",
    seriesKey,
    isEpisode,
  };
}

/**
 * 判定是否满足跳过片头条件：
 * - 视频总时长大于（片头+片尾+30秒），且大于 180 秒（防止短视频/剪辑误跳）；
 * - 片头时长有效 (> 0)；
 * - 当前播放时间处于片头开头附近（0 <= currentTime < 3s），避免用户手动倒带时被重复跳过；
 * - 本轮未曾跳过。
 */
export function canSkipIntro(
  currentTime: number,
  introSeconds: number,
  duration: number,
  alreadySkippedIntro: boolean
): boolean {
  if (alreadySkippedIntro) return false;
  if (!introSeconds || introSeconds <= 0) return false;
  if (!duration || isNaN(duration) || duration < Math.max(180, introSeconds + 30)) return false;

  // 仅在视频最开始前 3 秒内自动触发跳过
  return currentTime >= 0 && currentTime < 3;
}

/**
 * 判定是否满足跳过片尾条件：
 * - 视频总时长足够；
 * - 片尾时长有效 (> 0)；
 * - 当前播放时间已进入片尾区间（duration - currentTime <= outroSeconds），且距离视频真正结束大于 1 秒；
 * - 本轮未曾跳过片尾。
 */
export function canSkipOutro(
  currentTime: number,
  outroSeconds: number,
  duration: number,
  alreadySkippedOutro: boolean
): boolean {
  if (alreadySkippedOutro) return false;
  if (!outroSeconds || outroSeconds <= 0) return false;
  if (!duration || isNaN(duration) || duration < Math.max(180, outroSeconds + 30)) return false;

  const remaining = duration - currentTime;
  return remaining <= outroSeconds && remaining > 1;
}
