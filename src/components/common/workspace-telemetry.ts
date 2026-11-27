import type { DownloadTask } from "../../types";

const ACTIVE_TASK_STATUSES = new Set([
  "downloading",
  "connecting",
  "verifying",
  "extracting",
]);

/**
 * 计算连接数列中点亮的活动连接微点数量（基于真实连接/节点状态，不伪造数据）。
 */
export function computeActiveConnDots(task: DownloadTask, slots = 4): number {
  if (slots <= 0 || !ACTIVE_TASK_STATUSES.has(task.status)) {
    return 0;
  }

  if (task.task_kind === "bt") {
    const peers = Math.max(0, task.bt_runtime?.num_peers ?? task.active_connections ?? 0);
    if (peers === 0) return 0;
    if (peers <= 3) return Math.min(1, slots);
    if (peers <= 11) return Math.min(2, slots);
    if (peers <= 23) return Math.min(3, slots);
    return slots;
  }

  const active = Math.max(0, task.active_connections);
  if (active === 0) return 0;
  const total = Math.max(1, task.connection_count);
  return Math.max(1, Math.min(slots, Math.ceil((active / total) * slots)));
}

/**
 * 将真实速率采样序列归一化为像素柱高（全零时保持静态基线，不伪造波形）。
 */
export function computeSparkHeights(
  samples: readonly number[],
  minPx = 2,
  maxPx = 10
): number[] {
  if (samples.length === 0) return [];
  const safeMin = Math.max(1, minPx);
  const safeMax = Math.max(safeMin, maxPx);
  let peak = 0;
  for (const v of samples) {
    if (Number.isFinite(v) && v > peak) {
      peak = v;
    }
  }
  if (peak <= 0) {
    return samples.map(() => safeMin);
  }
  return samples.map((v) => {
    const safeVal = Number.isFinite(v) && v > 0 ? v : 0;
    if (safeVal === 0) return safeMin;
    const ratio = Math.min(1, safeVal / peak);
    return Math.round(safeMin + ratio * (safeMax - safeMin));
  });
}
