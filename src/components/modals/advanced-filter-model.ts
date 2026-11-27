import type { AdvancedFilter, Tag, TaskStatus } from "../../types";

export const PRIMARY_FILTER_STATUSES: ReadonlyArray<TaskStatus> = [
  "downloading",
  "queued",
  "paused",
  "completed",
  "failed",
  "cancelled",
];

export const EXTRA_FILTER_STATUSES: ReadonlyArray<TaskStatus> = [
  "scheduled",
  "verifying",
  "waiting-network",
  "remote-changed",
  "interrupted",
  "paused-by-low-disk",
  "paused-by-metered",
];

export const FILTER_STATUS_TONES: Record<
  TaskStatus,
  "accent" | "success" | "warning" | "danger" | "muted"
> = {
  queued: "muted",
  downloading: "accent",
  paused: "warning",
  completed: "success",
  failed: "danger",
  cancelled: "muted",
  scheduled: "warning",
  verifying: "accent",
  "waiting-network": "warning",
  "remote-changed": "danger",
  interrupted: "danger",
  "paused-by-low-disk": "warning",
  "paused-by-metered": "warning",
};

export const SIZE_PRESETS_MB: ReadonlyArray<{ label: string; minMb: number }> = [
  { label: ">100MB", minMb: 100 },
  { label: ">1GB", minMb: 1024 },
];

export type ActiveFilterChipKind =
  | "status"
  | "source"
  | "domain"
  | "date"
  | "size"
  | "tag";

export interface ActiveFilterChip {
  id: string;
  kind: ActiveFilterChipKind;
  value: string;
  label: string;
  color?: string;
}

export function countActiveFilterConditions(filter: AdvancedFilter): number {
  return (
    filter.statuses.length +
    filter.sources.length +
    (filter.domain.trim() ? 1 : 0) +
    (filter.dateFrom != null || filter.dateTo != null ? 1 : 0) +
    (filter.sizeMin != null || filter.sizeMax != null ? 1 : 0) +
    filter.tagIds.length
  );
}

export function countExtraSelectedStatuses(filter: AdvancedFilter): number {
  return filter.statuses.filter((status) =>
    EXTRA_FILTER_STATUSES.includes(status)
  ).length;
}

function formatDateDay(ts: number): string {
  const date = new Date(ts);
  const yyyy = date.getFullYear();
  const mm = String(date.getMonth() + 1).padStart(2, "0");
  const dd = String(date.getDate()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd}`;
}

export function buildActiveFilterChips(
  filter: AdvancedFilter,
  tags: ReadonlyArray<Tag>,
  resolvers: {
    statusLabel: (status: TaskStatus) => string;
    sourceLabel: (source: string) => string;
    domainPrefix: string;
    datePrefix: string;
    sizePrefix: string;
    tagPrefix: string;
  }
): ActiveFilterChip[] {
  const chips: ActiveFilterChip[] = [];

  for (const status of filter.statuses) {
    chips.push({
      id: `status:${status}`,
      kind: "status",
      value: status,
      label: resolvers.statusLabel(status),
    });
  }

  for (const source of filter.sources) {
    chips.push({
      id: `source:${source}`,
      kind: "source",
      value: source,
      label: resolvers.sourceLabel(source),
    });
  }

  const trimmedDomain = filter.domain.trim();
  if (trimmedDomain) {
    chips.push({
      id: `domain:${trimmedDomain}`,
      kind: "domain",
      value: trimmedDomain,
      label: `${resolvers.domainPrefix}: ${trimmedDomain}`,
    });
  }

  if (filter.dateFrom != null || filter.dateTo != null) {
    const start = filter.dateFrom != null ? formatDateDay(filter.dateFrom) : "…";
    const end = filter.dateTo != null ? formatDateDay(filter.dateTo) : "…";
    chips.push({
      id: `date:${start}-${end}`,
      kind: "date",
      value: "date",
      label: `${resolvers.datePrefix}: ${start} ~ ${end}`,
    });
  }

  if (filter.sizeMin != null || filter.sizeMax != null) {
    const minMb =
      filter.sizeMin != null
        ? Math.round((filter.sizeMin / (1024 * 1024)) * 10) / 10
        : 0;
    const maxMb =
      filter.sizeMax != null
        ? Math.round((filter.sizeMax / (1024 * 1024)) * 10) / 10
        : "∞";
    chips.push({
      id: `size:${minMb}-${maxMb}`,
      kind: "size",
      value: "size",
      label: `${resolvers.sizePrefix}: ${minMb}–${maxMb} MB`,
    });
  }

  for (const tagId of filter.tagIds) {
    const tag = tags.find((item) => item.id === tagId);
    chips.push({
      id: `tag:${tagId}`,
      kind: "tag",
      value: tagId,
      label: `${resolvers.tagPrefix}: ${tag?.name ?? tagId}`,
      color: tag?.color,
    });
  }

  return chips;
}

export function removeFilterChip(
  filter: AdvancedFilter,
  kind: ActiveFilterChipKind,
  value: string
): AdvancedFilter {
  switch (kind) {
    case "status":
      return {
        ...filter,
        statuses: filter.statuses.filter((status) => status !== value),
      };
    case "source":
      return {
        ...filter,
        sources: filter.sources.filter((source) => source !== value),
      };
    case "domain":
      return { ...filter, domain: "" };
    case "date":
      return { ...filter, dateFrom: null, dateTo: null };
    case "size":
      return { ...filter, sizeMin: null, sizeMax: null };
    case "tag":
      return {
        ...filter,
        tagIds: filter.tagIds.filter((tagId) => tagId !== value),
      };
  }
}

export function isSizePresetActive(
  filter: AdvancedFilter,
  minMb: number
): boolean {
  const targetBytes = minMb * 1024 * 1024;
  return filter.sizeMin === targetBytes && filter.sizeMax == null;
}

export function toggleSizePreset(
  filter: AdvancedFilter,
  minMb: number
): AdvancedFilter {
  if (isSizePresetActive(filter, minMb)) {
    return { ...filter, sizeMin: null, sizeMax: null };
  }
  return { ...filter, sizeMin: minMb * 1024 * 1024, sizeMax: null };
}

export function summarizeHistoryTasks(
  tasks: ReadonlyArray<{ total_bytes?: number; downloaded_bytes?: number }>,
  totalCount: number
): { matchedCount: number; totalCount: number; totalBytes: number } {
  let totalBytes = 0;
  for (const task of tasks) {
    const bytes = task.total_bytes || task.downloaded_bytes || 0;
    if (bytes > 0) totalBytes += bytes;
  }
  return {
    matchedCount: tasks.length,
    totalCount,
    totalBytes,
  };
}
