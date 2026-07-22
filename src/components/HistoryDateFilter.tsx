import { Select } from "./Select";

export type HistoryDatePreset = "all" | "today" | "7-days" | "30-days" | "custom";

export interface HistoryDateRange {
  preset: HistoryDatePreset;
  start: string;
  end: string;
}

export const defaultHistoryDateRange: HistoryDateRange = { preset: "all", start: "", end: "" };

function localDayStart(value: string) {
  if (!value) return undefined;
  const [year, month, day] = value.split("-").map(Number);
  if (!year || !month || !day) return undefined;
  return new Date(year, month - 1, day).getTime();
}

export function matchesHistoryDate(completedAt: number | undefined, range: HistoryDateRange, current = Date.now()) {
  if (!completedAt || range.preset === "all") return range.preset === "all";
  const now = new Date(current);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (range.preset === "today") return completedAt >= today;
  if (range.preset === "7-days") return completedAt >= today - 6 * 86_400_000;
  if (range.preset === "30-days") return completedAt >= today - 29 * 86_400_000;
  const start = localDayStart(range.start);
  const end = localDayStart(range.end);
  return (start === undefined || completedAt >= start)
    && (end === undefined || completedAt < end + 86_400_000);
}

const PRESET_OPTIONS: Array<{ value: HistoryDatePreset; label: string }> = [
  { value: "all", label: "全部时间" },
  { value: "today", label: "今天" },
  { value: "7-days", label: "最近 7 天" },
  { value: "30-days", label: "最近 30 天" },
  { value: "custom", label: "自定义范围" },
];

export function HistoryDateFilter({ value, onChange }: { value: HistoryDateRange; onChange: (next: HistoryDateRange) => void }) {
  return (
    <div className="history-filter-bar" aria-label="完成日期筛选">
      <span>完成日期</span>
      <Select
        value={value.preset}
        onChange={(nextPreset) => onChange({ ...value, preset: nextPreset as HistoryDatePreset })}
        options={PRESET_OPTIONS}
        ariaLabel="完成日期筛选"
      />
      {value.preset === "custom" && <>
        <input type="date" aria-label="开始日期" value={value.start} onChange={(event) => onChange({ ...value, start: event.target.value })} />
        <span>至</span>
        <input type="date" aria-label="结束日期" value={value.end} min={value.start || undefined} onChange={(event) => onChange({ ...value, end: event.target.value })} />
      </>}
    </div>
  );
}
