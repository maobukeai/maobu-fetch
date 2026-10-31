import type { ProgressWindowSnapshot, ProgressWindowTask } from "./progress-api";

export const COLLAPSED_ROWS = 3;
export const EXPANDED_ROWS = 6;

export function summaryPercent(snapshot: ProgressWindowSnapshot | null): number | null {
  const value = snapshot?.summary_percent;
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 100 ? value : null;
}

export function nonNegative(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

export function taskPercent(task: Pick<ProgressWindowTask, "total_bytes" | "downloaded_bytes" | "fetching_metadata">): number | null {
  if (task.fetching_metadata || !Number.isFinite(task.total_bytes) || task.total_bytes <= 0) return null;
  return Math.min(100, nonNegative(task.downloaded_bytes) / task.total_bytes * 100);
}

export function visibleTasks(snapshot: ProgressWindowSnapshot): ProgressWindowTask[] {
  return snapshot.tasks.slice(0, snapshot.expanded ? EXPANDED_ROWS : COLLAPSED_ROWS);
}

/** Listening starts before the initial invoke; a late initial result must not undo an event. */
export function newestSnapshot(current: ProgressWindowSnapshot | null, incoming: ProgressWindowSnapshot): ProgressWindowSnapshot {
  return current && incoming.revision < current.revision ? current : incoming;
}
