import { useState } from "react";
import { Save, X } from "lucide-react";
import { t } from "../../i18n";
import type { AdvancedFilter, QuickView, Tag, TaskStatus } from "../../types";
import { isAdvancedFilterEmpty } from "../../formatters";

export function AdvancedFilterPanel({
  value,
  onChange,
  tags,
  quickViews,
  onApplyQuickView,
  onSaveQuickView,
  onDeleteQuickView,
  onClear,
}: {
  value: AdvancedFilter;
  onChange: (next: AdvancedFilter) => void;
  tags: Tag[];
  quickViews: QuickView[];
  onApplyQuickView: (view: QuickView) => void;
  onSaveQuickView: (name: string) => void;
  onDeleteQuickView: (id: string) => void;
  onClear: () => void;
}) {
  const [saveName, setSaveName] = useState("");
  const [showSaveInput, setShowSaveInput] = useState(false);
  const allStatuses: TaskStatus[] = [
    "queued",
    "downloading",
    "paused",
    "completed",
    "failed",
    "cancelled",
    "scheduled",
    "verifying",
    "waiting-network",
    "remote-changed",
    "interrupted",
    "paused-by-low-disk",
    "paused-by-metered",
  ];

  const sourceKeys: Record<string, string> = {
    manual: "advancedFilter.sourceManual",
    extension: "advancedFilter.sourceExtension",
    "deep-link": "advancedFilter.sourceDeepLink",
    desktop: "advancedFilter.sourceDesktop",
    clipboard: "advancedFilter.sourceClipboard",
  };

  const toggleStatus = (status: TaskStatus) => {
    onChange({
      ...value,
      statuses: value.statuses.includes(status)
        ? value.statuses.filter((s) => s !== status)
        : [...value.statuses, status],
    });
  };
  const toggleTag = (id: string) => {
    onChange({
      ...value,
      tagIds: value.tagIds.includes(id)
        ? value.tagIds.filter((t) => t !== id)
        : [...value.tagIds, id],
    });
  };
  const toggleSource = (src: string) => {
    onChange({
      ...value,
      sources: value.sources.includes(src)
        ? value.sources.filter((s) => s !== src)
        : [...value.sources, src],
    });
  };
  const isEmpty = isAdvancedFilterEmpty(value);

  return (
    <div
      className="advanced-filter-panel"
      role="region"
      aria-label={t("advancedFilter.title")}
    >
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">
          {t("advancedFilter.status")}
        </span>
        <div className="advanced-filter-chips">
          {allStatuses.map((status) => (
            <button
              key={status}
              className={`filter-chip${
                value.statuses.includes(status) ? " active" : ""
              }`}
              onClick={() => toggleStatus(status)}
              type="button"
              aria-pressed={value.statuses.includes(status)}
            >
              {t("statusFilter." + status)}
            </button>
          ))}
        </div>
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">
          {t("advancedFilter.source")}
        </span>
        <div className="advanced-filter-chips">
          {Object.keys(sourceKeys).map((src) => (
            <button
              key={src}
              className={`filter-chip${
                value.sources.includes(src) ? " active" : ""
              }`}
              onClick={() => toggleSource(src)}
              type="button"
              aria-pressed={value.sources.includes(src)}
            >
              {t(sourceKeys[src])}
            </button>
          ))}
        </div>
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">
          {t("advancedFilter.domain")}
        </span>
        <input
          type="text"
          placeholder={t("advancedFilter.domainPlaceholder")}
          value={value.domain}
          onChange={(e) => onChange({ ...value, domain: e.target.value })}
          aria-label={t("advancedFilter.domain")}
        />
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">
          {t("advancedFilter.addedDate")}
        </span>
        <input
          type="date"
          aria-label={t("advancedFilter.startDate")}
          value={
            value.dateFrom
              ? new Date(value.dateFrom).toISOString().slice(0, 10)
              : ""
          }
          onChange={(e) => {
            const v = e.target.value;
            onChange({
              ...value,
              dateFrom: v ? new Date(v + "T00:00:00").getTime() : null,
            });
          }}
        />
        <span>{t("historyFilter.to")}</span>
        <input
          type="date"
          aria-label={t("advancedFilter.endDate")}
          value={
            value.dateTo
              ? new Date(value.dateTo).toISOString().slice(0, 10)
              : ""
          }
          onChange={(e) => {
            const v = e.target.value;
            onChange({
              ...value,
              dateTo: v ? new Date(v + "T23:59:59.999").getTime() : null,
            });
          }}
        />
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">
          {t("advancedFilter.sizeRange")}
        </span>
        <input
          type="number"
          min={0}
          placeholder={t("advancedFilter.sizeMinPlaceholder")}
          value={
            value.sizeMin != null
              ? (value.sizeMin / (1024 * 1024)).toString()
              : ""
          }
          onChange={(e) => {
            const v = e.target.value
              ? Number(e.target.value) * 1024 * 1024
              : null;
            onChange({
              ...value,
              sizeMin: v != null && Number.isFinite(v) ? Math.max(0, v) : null,
            });
          }}
          aria-label={t("advancedFilter.sizeMinPlaceholder")}
        />
        <span>{t("historyFilter.to")}</span>
        <input
          type="number"
          min={0}
          placeholder={t("advancedFilter.sizeMaxPlaceholder")}
          value={
            value.sizeMax != null
              ? (value.sizeMax / (1024 * 1024)).toString()
              : ""
          }
          onChange={(e) => {
            const v = e.target.value
              ? Number(e.target.value) * 1024 * 1024
              : null;
            onChange({
              ...value,
              sizeMax: v != null && Number.isFinite(v) ? Math.max(0, v) : null,
            });
          }}
          aria-label={t("advancedFilter.sizeMaxPlaceholder")}
        />
      </div>
      {tags.length > 0 && (
        <div className="advanced-filter-row">
          <span className="advanced-filter-label">
            {t("advancedFilter.tags")}
          </span>
          <div className="advanced-filter-chips">
            {tags.map((tag) => (
              <button
                key={tag.id}
                className={`filter-chip with-color${
                  value.tagIds.includes(tag.id) ? " active" : ""
                }`}
                style={
                  value.tagIds.includes(tag.id)
                    ? { background: tag.color, borderColor: tag.color }
                    : undefined
                }
                onClick={() => toggleTag(tag.id)}
                type="button"
                aria-pressed={value.tagIds.includes(tag.id)}
                title={tag.name}
              >
                {tag.name}
              </button>
            ))}
          </div>
        </div>
      )}
      <div className="advanced-filter-actions">
        <button onClick={onClear} disabled={isEmpty} type="button">
          {t("advancedFilter.clearFilter")}
        </button>
        {showSaveInput ? (
          <>
            <input
              type="text"
              placeholder={t("advancedFilter.quickViewName")}
              value={saveName}
              onChange={(e) => setSaveName(e.target.value)}
              maxLength={20}
              aria-label={t("advancedFilter.quickViewName")}
            />
            <button
              onClick={() => {
                if (saveName.trim()) {
                  onSaveQuickView(saveName.trim());
                  setSaveName("");
                  setShowSaveInput(false);
                }
              }}
              disabled={!saveName.trim() || isEmpty}
              type="button"
            >
              {t("common.save")}
            </button>
            <button
              onClick={() => {
                setShowSaveInput(false);
                setSaveName("");
              }}
              type="button"
            >
              {t("common.cancel")}
            </button>
          </>
        ) : (
          <button
            onClick={() => setShowSaveInput(true)}
            disabled={isEmpty}
            type="button"
          >
            <Save size={12} />
            {t("advancedFilter.saveAsQuickView")}
          </button>
        )}
      </div>
      {quickViews.length > 0 && (
        <div className="advanced-filter-row quick-views">
          <span className="advanced-filter-label">
            {t("advancedFilter.quickViews")}
          </span>
          <div className="advanced-filter-chips">
            {quickViews.map((qv) => (
              <span key={qv.id} className="quick-view-chip">
                <button
                  className="filter-chip"
                  onClick={() => onApplyQuickView(qv)}
                  type="button"
                  title={t("advancedFilter.applyQuickView", { name: qv.name })}
                >
                  {qv.name}
                </button>
                <button
                  className="quick-view-delete"
                  onClick={() => onDeleteQuickView(qv.id)}
                  type="button"
                  title={t("advancedFilter.deleteQuickView")}
                  aria-label={t("advancedFilter.deleteQuickView")}
                >
                  <X size={10} />
                </button>
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
