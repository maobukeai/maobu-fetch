/**
 * 猫步看图器（Maobu Image Viewer）纯逻辑状态与变换计算模型
 */

export type ViewerFilterMode = "none" | "contrast" | "grayscale" | "invert";

export const VIEWER_MIN_SCALE = 0.1;
export const VIEWER_MAX_SCALE = 10;

export const VIEWER_FILTER_LABELS: Record<ViewerFilterMode, string> = {
  none: "",
  contrast: "高对比",
  grayscale: "黑白",
  invert: "反色",
};

export interface ViewerPosition {
  x: number;
  y: number;
}

export interface ViewerStageRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface CursorAnchoredZoomInput {
  scale: number;
  position: ViewerPosition;
  factor: number;
  pointerX: number;
  pointerY: number;
  stageRect: ViewerStageRect;
}

/**
 * 将缩放比例限制在 [0.1, 10] 并保留两位小数。
 */
export function clampScale(scale: number): number {
  if (!Number.isFinite(scale)) return 1;
  const rounded = parseFloat(scale.toFixed(2));
  return Math.min(VIEWER_MAX_SCALE, Math.max(VIEWER_MIN_SCALE, rounded));
}

/**
 * 滤镜循环切换：原图 -> 高对比 -> 黑白 -> 反色 -> 原图。
 */
export function nextFilterMode(current: ViewerFilterMode): {
  mode: ViewerFilterMode;
  toast: string;
} {
  switch (current) {
    case "none":
      return { mode: "contrast", toast: "滤镜：高对比" };
    case "contrast":
      return { mode: "grayscale", toast: "滤镜：黑白" };
    case "grayscale":
      return { mode: "invert", toast: "滤镜：反色" };
    case "invert":
    default:
      return { mode: "none", toast: "滤镜：原图" };
  }
}

/**
 * 以鼠标指针所在视口坐标为锚点计算缩放后的新比例与平移补偿，
 * 保证缩放前后指针下方的像素在屏幕上的位置保持不变。
 */
export function computeCursorAnchoredZoom(input: CursorAnchoredZoomInput): {
  scale: number;
  position: ViewerPosition;
} {
  const { scale, position, factor, pointerX, pointerY, stageRect } = input;
  const nextScale = clampScale(scale * factor);
  if (nextScale === scale || scale <= 0) {
    return { scale, position };
  }

  const cx = pointerX - (stageRect.left + stageRect.width / 2);
  const cy = pointerY - (stageRect.top + stageRect.height / 2);
  const ratio = nextScale / scale;

  return {
    scale: nextScale,
    position: {
      x: parseFloat((cx - (cx - position.x) * ratio).toFixed(2)),
      y: parseFloat((cy - (cy - position.y) * ratio).toFixed(2)),
    },
  };
}

function gcd(a: number, b: number): number {
  let x = Math.abs(Math.round(a));
  let y = Math.abs(Math.round(b));
  while (y !== 0) {
    const t = x % y;
    x = y;
    y = t;
  }
  return x || 1;
}

/**
 * 计算并格式化图片宽高比（常见比例返回整数比，复杂比例返回保留两位小数的比值）。
 */
export function formatAspectRatio(width: number, height: number): string {
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    return "—";
  }
  const g = gcd(width, height);
  const w = Math.round(width) / g;
  const h = Math.round(height) / g;
  if (w <= 32 && h <= 32) {
    return `${w}:${h}`;
  }
  return `${(width / height).toFixed(2)}:1`;
}

/**
 * 解析图片展示 URL：支持内联 data/blob/http、Tauri convertFileSrc 以及 stream 备用协议。
 */
export function computeViewerImageUrl(
  path: string,
  fallback: boolean,
  desktop: boolean,
  convertFn?: (p: string) => string
): string {
  if (!path) return "";
  if (/^(data:|blob:|https?:)/i.test(path)) {
    return path;
  }
  const streamUrl = `stream://localhost?file=${encodeURIComponent(path)}`;
  if (fallback) {
    return streamUrl;
  }
  if (desktop && convertFn) {
    try {
      return convertFn(path);
    } catch {
      return streamUrl;
    }
  }
  return streamUrl;
}
