import { X } from "lucide-react";
import { t } from "../../i18n";
import type { AdvancedFilter, Tag } from "../../types";
import {
  buildActiveFilterChips,
  type ActiveFilterChipKind,
} from "./advanced-filter-model";

const SOURCE_KEY_MAP: Record<string, string> = {
  manual: "advancedFilter.sourceManual",
  extension: "advancedFilter.sourceExtension",
  "deep-link": "advancedFilter.sourceDeepLink",
  desktop: "advancedFilter.sourceDesktop",
  clipboard: "advancedFilter.sourceClipboard",
};

export function ActiveFilterSummaryBar({
  value,
  tags,
  matchedCount,
  totalCount,
  onRemoveChip,
  onExpand,
  onClear,
}: {
  value: AdvancedFilter;
  tags: ReadonlyArray<Tag>;
  matchedCount?: number;
  totalCount?: number;
  onRemoveChip: (kind: ActiveFilterChipKind, chipValue: string) => void;
  onExpand: () => void;
  onClear: () => void;
}) {
  const chips = buildActiveFilterChips(value, tags, {
    statusLabel: (status) => t(`statusFilter.${status}`),
    sourceLabel: (src) => t(SOURCE_KEY_MAP[src] ?? src),
    domainPrefix: t("advancedFilter.domain"),
    datePrefix: t("advancedFilter.addedDate"),
    sizePrefix: t("advancedFilter.sizeRange"),
    tagPrefix: t("advancedFilter.tags"),
  });

  if (chips.length === 0) return null;

  return (
    <div
      className="active-filter-strip"
      role="region"
      aria-label={t("advancedFilter.activePrefix")}
    >
      <span className="af-eyebrow">{t("advancedFilter.activePrefix")}</span>
      <div className="advanced-filter-chips">
        {chips.map((chip) => (
          <button
            key={chip.id}
            type="button"
            className={
              chip.color
                ? "filter-chip with-color active"
                : "filter-chip active"
            }
            style={
              chip.color
                ? { background: chip.color, borderColor: chip.color }
                : undefined
            }
            onClick={() => onRemoveChip(chip.kind, chip.value)}
            title={t("advancedFilter.clearShort")}
          >
            <span>{chip.label}</span>
            <X size={10} aria-hidden="true" />
          </button>
        ))}
      </div>
      <div className="af-strip-actions">
        {matchedCount != null && totalCount != null && (
          <span className="af-match-count active">
            {t("advancedFilter.matchedCount", {
              matched: matchedCount,
              total: totalCount,
            })}
          </span>
        )}
        <button type="button" className="af-inline-btn" onClick={onExpand}>
          {t("advancedFilter.expandPanel")}
        </button>
        <button type="button" className="af-inline-btn" onClick={onClear}>
          {t("advancedFilter.clearShort")}
        </button>
      </div>
    </div>
  );
}
