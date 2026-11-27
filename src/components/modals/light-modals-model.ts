export interface SpeedLimitPreset {
  kb: number;
  label?: string;
  labelKey?: string;
}

export const SPEED_LIMIT_PRESETS: ReadonlyArray<SpeedLimitPreset> = [
  { kb: 0, labelKey: "details.noSpeedLimit" },
  { kb: 512, label: "512 KB/s" },
  { kb: 1024, label: "1 MB/s" },
  { kb: 2048, label: "2 MB/s" },
  { kb: 5120, label: "5 MB/s" },
  { kb: 10240, label: "10 MB/s" },
];

export type RenameValidationError =
  | "empty"
  | "invalid-chars"
  | "path-traversal"
  | "too-long";

export interface ClampedMenuPosition {
  left: number;
  top: number;
  originX: "left" | "right";
  originY: "top" | "bottom";
}

const WINDOWS_INVALID_FILENAME_CHARS = /[<>:"/\\|?*\x00-\x1F]/;
const PATH_TRAVERSAL_PATTERN = /(^|[\\/])\.\.([\\/]|$)/;
const MAX_RENAME_LENGTH = 240;

/**
 * Formats a per-task speed limit in bytes/sec for compact badges and menu subtitles.
 */
export function formatSpeedLimitBadge(
  limitBytesPerSec: number,
  unlimitedLabel: string
): string {
  if (!Number.isFinite(limitBytesPerSec) || limitBytesPerSec <= 0) {
    return unlimitedLabel;
  }
  const kb = Math.max(1, Math.round(limitBytesPerSec / 1024));
  if (kb < 1024) {
    return `${kb} KB/s`;
  }
  const mb = kb / 1024;
  return Number.isInteger(mb) ? `${mb} MB/s` : `${mb.toFixed(1)} MB/s`;
}

/**
 * Validates a renamed task filename without echoing raw untrusted input back into errors.
 */
export function validateRenameFilename(
  raw: string
): RenameValidationError | null {
  const trimmed = raw.trim();
  if (!trimmed) {
    return "empty";
  }
  if (
    trimmed === "." ||
    trimmed === ".." ||
    PATH_TRAVERSAL_PATTERN.test(trimmed)
  ) {
    return "path-traversal";
  }
  if (WINDOWS_INVALID_FILENAME_CHARS.test(trimmed)) {
    return "invalid-chars";
  }
  if (trimmed.length > MAX_RENAME_LENGTH) {
    return "too-long";
  }
  return null;
}

/**
 * Clamps context menu coordinates to the viewport and returns matching transform origins.
 */
export function clampMenuPosition(
  x: number,
  y: number,
  menuWidth: number,
  menuHeight: number,
  viewportWidth: number,
  viewportHeight: number,
  padding = 8
): ClampedMenuPosition {
  const maxLeft = Math.max(padding, viewportWidth - menuWidth - padding);
  const maxTop = Math.max(padding, viewportHeight - menuHeight - padding);
  const overflowsRight = x + menuWidth + padding > viewportWidth;
  const overflowsBottom = y + menuHeight + padding > viewportHeight;

  const left = overflowsRight
    ? Math.max(padding, Math.min(x - menuWidth, maxLeft))
    : Math.max(padding, Math.min(x, maxLeft));
  const top = overflowsBottom
    ? Math.max(padding, Math.min(y - menuHeight, maxTop))
    : Math.max(padding, Math.min(y, maxTop));

  return {
    left: Math.round(left),
    top: Math.round(top),
    originX: overflowsRight ? "right" : "left",
    originY: overflowsBottom ? "bottom" : "top",
  };
}
