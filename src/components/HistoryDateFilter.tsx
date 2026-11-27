import { formatBytes } from "../formatters";
import { t } from "../i18n";
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

export function HistoryDateFilter({
  value,
  onChange,
  matchedCount,
  totalCount,
  totalBytes,
}: {
  value: HistoryDateRange;
  onChange: (next: HistoryDateRange) => void;
  matchedCount?: number;
  totalCount?: number;
  totalBytes?: number;
}) {
  const options: Array<{ value: HistoryDatePreset; label: string }> = [
    { value: "all", label: t("historyFilter.allTime") },
    { value: "today", label: t("historyFilter.today") },
    { value: "7-days", label: t("historyFilter.last7Days") },
    { value: "30-days", label: t("historyFilter.last30Days") },
    { value: "custom", label: t("historyFilter.customRange") },
  ];

  return (
    <div className="history-filter-bar" aria-label="完成日期筛选">
      <div className="history-filter-controls">
        <span>{t("historyFilter.completionDate")}</span>
        <Select
          value={value.preset}
          onChange={(nextPreset) =>
            onChange({ ...value, preset: nextPreset as HistoryDatePreset })
          }
          options={options}
          ariaLabel="完成日期筛选"
        />
        {value.preset === "custom" && (
          <>
            <input
              type="date"
              aria-label={t("historyFilter.startDate")}
              value={value.start}
              onChange={(event) =>
                onChange({ ...value, start: event.target.value })
              }
            />
            <span>{t("historyFilter.to")}</span>
            <input
              type="date"
              aria-label={t("historyFilter.endDate")}
              value={value.end}
              min={value.start || undefined}
              onChange={(event) =>
                onChange({ ...value, end: event.target.value })
              }
            />
          </>
        )}
      </div>

      {(matchedCount != null || value.preset !== "all") && (
        <div className="history-filter-telemetry">
          {matchedCount != null && totalCount != null && totalBytes != null && (
            <span className="history-filter-stats">
              {matchedCount === totalCount
                ? t("historyFilter.summaryAll", {
                    count: matchedCount,
                    size: formatBytes(totalBytes),
                  })
                : t("historyFilter.summary", {
                    matched: matchedCount,
                    total: totalCount,
                    size: formatBytes(totalBytes),
                  })}
            </span>
          )}
          {value.preset !== "all" && (
            <button
              type="button"
              className="history-filter-reset"
              onClick={() => onChange(defaultHistoryDateRange)}
            >
              {t("historyFilter.reset")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
