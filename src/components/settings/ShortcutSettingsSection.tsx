import { useMemo, useState } from "react";
import { AlertCircle, RotateCcw } from "lucide-react";
import { t } from "../../i18n";
import type { ShortcutKeys } from "../../types";
import { DEFAULT_SHORTCUTS, parseShortcutEvent } from "../../formatters";

export function ShortcutSettingsSection({
  value,
  onChange,
  notify,
}: {
  value: ShortcutKeys;
  onChange: (value: ShortcutKeys) => void;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [recordingAction, setRecordingAction] = useState<keyof ShortcutKeys | null>(null);

  const actionKeys: Array<{ key: keyof ShortcutKeys; label: string }> = [
    { key: "new_task", label: t("shortcuts.actionNewTask") },
    { key: "select_all", label: t("shortcuts.actionSelectAll") },
    { key: "copy_url", label: t("shortcuts.actionCopyUrl") },
    { key: "open_folder", label: t("shortcuts.actionOpenFolder") },
    { key: "toggle_pause", label: t("shortcuts.actionTogglePause") },
    { key: "rename_task", label: t("shortcuts.actionRenameTask") },
    { key: "delete_task", label: t("shortcuts.actionDeleteTask") },
    { key: "delete_file", label: t("shortcuts.actionDeleteFile") },
  ];

  const conflicts = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const item of actionKeys) {
      const val = (value[item.key] || "").toLowerCase();
      if (!val) continue;
      const existing = map.get(val) || [];
      existing.push(item.label);
      map.set(val, existing);
    }
    const conflictMsgs: string[] = [];
    for (const [key, labels] of map.entries()) {
      if (labels.length > 1) {
        conflictMsgs.push(`${labels.join(" / ")} (${key.toUpperCase()})`);
      }
    }
    return conflictMsgs;
  }, [value]);

  const handleKeyDown = (actionKey: keyof ShortcutKeys, event: React.KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      setRecordingAction(null);
      return;
    }

    const nativeEvent = event.nativeEvent;
    const keyCombo = parseShortcutEvent(nativeEvent);

    if (["Ctrl", "Shift", "Alt"].includes(keyCombo)) {
      return;
    }

    onChange({ ...value, [actionKey]: keyCombo });
    setRecordingAction(null);
  };

  const handleResetDefaults = () => {
    onChange(DEFAULT_SHORTCUTS);
    notify(t("shortcuts.resetDefaultsSuccess"));
  };

  return (
    <div className="shortcuts-settings-section">
      <div className="shortcuts-section-header">
        <p className="settings-note" style={{ margin: 0 }}>
          {t("shortcuts.desc")}
        </p>
        <button
          className="secondary reset-btn"
          type="button"
          onClick={handleResetDefaults}
        >
          <RotateCcw size={13} />
          <span>{t("shortcuts.resetDefaults")}</span>
        </button>
      </div>

      {conflicts.length > 0 && (
        <div className="shortcut-conflict-warning">
          <AlertCircle size={14} />
          <span>{t("shortcuts.conflictWarning", { keys: conflicts.join("; ") })}</span>
        </div>
      )}

      <div className="shortcuts-list">
        {actionKeys.map(({ key, label }) => {
          const isRecording = recordingAction === key;
          const currentCombo = value[key] || DEFAULT_SHORTCUTS[key];

          return (
            <div key={key} className="shortcut-item-row">
              <span className="shortcut-action-label">{label}</span>
              <div className="shortcut-recorder-box">
                <button
                  type="button"
                  className={`shortcut-recorder-btn ${isRecording ? "recording" : ""}`}
                  onClick={() => setRecordingAction(isRecording ? null : key)}
                  onKeyDown={(e) => isRecording && handleKeyDown(key, e)}
                  tabIndex={0}
                >
                  {isRecording ? t("shortcuts.recordingHint") : currentCombo}
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
