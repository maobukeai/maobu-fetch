import { t } from "../../i18n";
import type { SettingsSectionId } from "./settings-fusion";

export interface SettingsHeaderAnchor {
  id: string;
  label: string;
}

export function SettingsFusionHeader({
  groupLabel,
  section,
  title,
  subtitle,
  anchors,
  activeAnchor,
  onSelectAnchor,
}: {
  groupLabel: string;
  section: SettingsSectionId;
  title: string;
  subtitle?: string;
  anchors: ReadonlyArray<SettingsHeaderAnchor>;
  activeAnchor: string | null;
  onSelectAnchor: (anchorId: string) => void;
}) {
  return (
    <header className="settings-fusion-header" data-tauri-drag-region>
      <div className="settings-fusion-header-main" data-tauri-drag-region>
        <div className="settings-fusion-eyebrow" data-tauri-drag-region>
          {groupLabel} · {section.toUpperCase()}
        </div>
        <div className="settings-title" data-tauri-drag-region>
          <h1 data-tauri-drag-region>{title}</h1>
          {subtitle && <span className="settings-fusion-desc">{subtitle}</span>}
        </div>
      </div>

      {anchors.length > 1 && (
        <div
          className="settings-anchor-strip"
          data-magnetic-group="pill"
          role="tablist"
          aria-label={title}
        >
          {anchors.map((anchor) => {
            const isActive = activeAnchor === anchor.id;
            return (
              <button
                key={anchor.id}
                type="button"
                role="tab"
                aria-selected={isActive}
                className={
                  isActive
                    ? "settings-anchor-pill active"
                    : "settings-anchor-pill"
                }
                onClick={() => onSelectAnchor(anchor.id)}
              >
                {anchor.label}
              </button>
            );
          })}
        </div>
      )}
    </header>
  );
}

export function SettingsFloatingFooter({
  dirtyCount,
  onRevert,
  onCancel,
  onSave,
}: {
  dirtyCount: number;
  onRevert: () => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const isDirty = dirtyCount > 0;
  return (
    <div className="settings-capsule-dock">
      <div
        className={
          isDirty
            ? "settings-floating-capsule dirty"
            : "settings-floating-capsule"
        }
      >
        <span className="settings-dirty-status">
          <i className="settings-dirty-dot" aria-hidden="true" />
          <span>
            {isDirty
              ? t("settings.statusDirtyCount", { count: dirtyCount })
              : t("settings.statusSynced")}
          </span>
        </span>
        <div className="dialog-actions settings-actions">
          {isDirty && (
            <button
              type="button"
              className="settings-revert-btn"
              onClick={onRevert}
            >
              {t("settings.revertChanges")}
            </button>
          )}
          <button type="button" onClick={onCancel}>
            {t("common.cancel")}
          </button>
          <button type="button" className="primary" onClick={onSave}>
            {t("settings.saveSettings")}
          </button>
        </div>
      </div>
    </div>
  );
}
