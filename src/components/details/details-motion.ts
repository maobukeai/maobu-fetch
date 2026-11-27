import type { TaskStatus } from "../../types";

export type DetailsTabKey = "info" | "diagnosis" | "precheck" | "connections";

export type DetailsTabSlideDirection = "left" | "right" | "none";

export type DetailsStatusTone =
  | "accent"
  | "success"
  | "danger"
  | "warning"
  | "muted";

export type DetailsDrawerPhase = "closed" | "opening" | "open" | "closing";

export const DETAILS_DRAWER_OPEN_MS = 210;
export const DETAILS_DRAWER_CLOSE_MS = 170;
export const DETAILS_DRAWER_UNMOUNT_BUFFER_MS = 20;

/** Return unmount delay (ms) after drawer collapse animation completes. */
export function getDetailsDrawerUnmountDelay(reducedMotion: boolean): number {
  if (reducedMotion) return 0;
  return DETAILS_DRAWER_CLOSE_MS + DETAILS_DRAWER_UNMOUNT_BUFFER_MS;
}

export interface SpeedSparkGeometry {
  peak: number;
  latest: number;
  linePath: string;
  areaPath: string;
  lastYPercent: number;
  isLive: boolean;
}

export const DETAILS_TAB_ORDER: readonly DetailsTabKey[] = [
  "info",
  "diagnosis",
  "precheck",
  "connections",
];

/** Compute horizontal slide direction when switching between Details tabs. */
export function getTabSlideDirection(
  prev: DetailsTabKey,
  next: DetailsTabKey
): DetailsTabSlideDirection {
  if (prev === next) return "none";
  const prevIndex = DETAILS_TAB_ORDER.indexOf(prev);
  const nextIndex = DETAILS_TAB_ORDER.indexOf(next);
  if (prevIndex < 0 || nextIndex < 0) return "none";
  return nextIndex > prevIndex ? "right" : "left";
}

/** Map task status and parsing state to a semantic badge tone. */
export function getDetailsStatusTone(
  status: TaskStatus,
  isParsing = false
): DetailsStatusTone {
  if (isParsing || status === "downloading") {
    return "accent";
  }
  if (status === "completed") {
    return "success";
  }
  if (
    status === "failed" ||
    status === "interrupted" ||
    status === "remote-changed"
  ) {
    return "danger";
  }
  if (
    status === "paused" ||
    status === "paused-by-metered" ||
    status === "paused-by-low-disk" ||
    status === "waiting-network" ||
    status === "queued" ||
    status === "scheduled"
  ) {
    return "warning";
  }
  return "muted";
}

/** Compute SVG line/area geometry and live end-dot vertical percentage for real speed samples. */
export function computeSpeedSparkGeometry(
  samples: number[],
  width: number,
  height: number
): SpeedSparkGeometry {
  const safeWidth = Math.max(1, width);
  const safeHeight = Math.max(8, height);
  if (samples.length === 0) {
    return {
      peak: 0,
      latest: 0,
      linePath: "",
      areaPath: "",
      lastYPercent: 50,
      isLive: false,
    };
  }

  const normalized = samples.map((value) =>
    Number.isFinite(value) && value > 0 ? value : 0
  );
  const rawPeak = Math.max(...normalized);
  const scaleMax = Math.max(rawPeak, 1);
  const latest = normalized[normalized.length - 1];

  if (normalized.length < 2) {
    const y = safeHeight - 2 - (latest / scaleMax) * (safeHeight - 6);
    const lastYPercent = Math.min(100, Math.max(0, (y / safeHeight) * 100));
    return {
      peak: rawPeak,
      latest,
      linePath: "",
      areaPath: "",
      lastYPercent,
      isLive: latest > 0,
    };
  }

  const step = safeWidth / (normalized.length - 1);
  let lastY = safeHeight - 2;
  const points = normalized.map((value, index) => {
    const x = index * step;
    const y = safeHeight - 2 - (value / scaleMax) * (safeHeight - 6);
    if (index === normalized.length - 1) {
      lastY = y;
    }
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });

  const linePath = `M ${points.join(" L ")}`;
  const areaPath = `${linePath} L ${safeWidth},${safeHeight} L 0,${safeHeight} Z`;
  const lastYPercent = Math.min(100, Math.max(0, (lastY / safeHeight) * 100));

  return {
    peak: rawPeak,
    latest,
    linePath,
    areaPath,
    lastYPercent,
    isLive: latest > 0,
  };
}
