export type PlayMode = "sequence" | "loop" | "single" | "shuffle";

export type PlaylistSortType =
  | "default"
  | "name-asc"
  | "name-desc"
  | "size-desc"
  | "size-asc";

export const PLAYBACK_RATES = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0] as const;

export const SUBTITLE_COLOR_PRESETS: ReadonlyArray<{
  color: string;
  label: string;
}> = [
  { color: "#ffffff", label: "纯白" },
  { color: "#fde047", label: "暖黄" },
  { color: "#60a5fa", label: "晴蓝" },
  { color: "#4ade80", label: "翠绿" },
  { color: "#f472b6", label: "珊瑚" },
];

export interface SeekHoverResult {
  ratio: number;
  clampedX: number;
  time: number;
}

/**
 * Formats playback seconds into MM:SS or HH:MM:SS.
 */
export function formatPlayerTime(seconds: number, forceHours = false): string {
  if (!Number.isFinite(seconds) || seconds < 0) {
    return forceHours ? "00:00:00" : "00:00";
  }
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;

  const mm = m.toString().padStart(2, "0");
  const ss = s.toString().padStart(2, "0");

  if (h > 0 || forceHours) {
    const hh = h.toString().padStart(2, "0");
    return `${hh}:${mm}:${ss}`;
  }
  return `${mm}:${ss}`;
}

/**
 * Formats the speed toggle label without clutter at 1.0x.
 */
export function formatPlaybackRateLabel(rate: number): string {
  if (!Number.isFinite(rate) || Math.abs(rate - 1.0) < 0.001) {
    return "倍速";
  }
  return `${rate}x`;
}

/**
 * Computes relative seek ratio, clamped tooltip X coordinate inside the rail box, and target time.
 */
export function computeSeekHover(
  clientX: number,
  rectLeft: number,
  rectWidth: number,
  duration: number,
  tooltipHalfWidth = 28
): SeekHoverResult {
  if (!Number.isFinite(rectWidth) || rectWidth <= 0) {
    return { ratio: 0, clampedX: 0, time: 0 };
  }
  const rawX = clientX - rectLeft;
  const clampedRaw = Math.max(0, Math.min(rectWidth, rawX));
  const ratio = clampedRaw / rectWidth;
  const half = Math.min(Math.max(0, tooltipHalfWidth), rectWidth / 2);
  const clampedX = Math.max(half, Math.min(rectWidth - half, clampedRaw));
  const time = Number.isFinite(duration) && duration > 0 ? ratio * duration : 0;

  return {
    ratio,
    clampedX: Math.round(clampedX * 100) / 100,
    time,
  };
}
