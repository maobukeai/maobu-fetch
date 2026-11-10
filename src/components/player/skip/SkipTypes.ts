/**
 * 猫步播放器（Maobu Player）跳过片头片尾与剧集看剧助手类型定义
 */

export interface SeriesSkipSetting {
  introSeconds: number;
  outroSeconds: number;
  updatedAt: number;
}

export interface SkipIntroOutroConfig {
  /** 是否总开启跳过功能 */
  enabled: boolean;
  /** 看剧时是否默认自动识别剧集/章节并跳过 */
  autoDetect: boolean;
  /** 默认片头跳过时长（秒，默认 90） */
  defaultIntroSeconds: number;
  /** 默认片尾跳过时长（秒，默认 90） */
  defaultOutroSeconds: number;
  /** 到达片尾时是否自动播放下一集 */
  autoPlayNextOnOutro: boolean;
  /** 按剧集目录/系列专属记忆的跳过设置 */
  seriesSettings: Record<string, SeriesSkipSetting>;
}

export type SkipSourceType = "chapter" | "series" | "default" | "disabled";

export interface ResolvedSkipSettings {
  introSeconds: number;
  outroSeconds: number;
  enabled: boolean;
  autoDetect: boolean;
  autoPlayNextOnOutro: boolean;
  source: SkipSourceType;
  sourceLabel: string;
  seriesKey: string;
  isEpisode: boolean;
}

export interface ChapterMatch {
  introSeconds?: number;
  outroSeconds?: number;
  introTitle?: string;
  outroTitle?: string;
}
