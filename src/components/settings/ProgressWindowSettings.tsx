import { useRef, useState } from "react";
import { ExternalLink } from "lucide-react";
import { t } from "../../i18n";
import type { AppSettings } from "../../types";
import { progressApi } from "../progress/progress-api";
import { SettingsGroup, Toggle } from "../common/FormComponents";

export function ProgressWindowSettings({ value, savedEnabled, onChange, notify }: {
  value: AppSettings;
  savedEnabled: boolean;
  onChange: (patch: Partial<AppSettings>) => void;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const restore = async () => {
    if (busyRef.current || !savedEnabled || value.progress_window_enabled === false) return;
    busyRef.current = true;
    setBusy(true);
    try {
      const snapshot = await progressApi.restore();
      if (snapshot.error || !snapshot.enabled) {
        notify(t("progressWindow.restoreError"), "error");
      } else {
        const hasTasks = snapshot.total_count > 0 || (snapshot.completed_count ?? 0) > 0;
        notify(t(hasTasks ? "progressWindow.restored" : "progressWindow.emptyRestore"));
      }
    } catch {
      notify(t("progressWindow.restoreError"), "error");
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  return <SettingsGroup title={t("progressWindow.settingsTitle")}>
    <div className="settings-group-content">
      <Toggle label={t("progressWindow.enabledLabel")} sub={t("progressWindow.enabledHint")}
        checked={value.progress_window_enabled ?? true}
        onChange={(enabled) => onChange({ progress_window_enabled: enabled })} />
      <Toggle label={t("progressWindow.topLabel")} sub={t("progressWindow.topHint")}
        checked={value.progress_window_always_on_top ?? true}
        onChange={(pinned) => onChange({ progress_window_always_on_top: pinned })} />
      <div className="setting-row">
        <div><strong>{t("progressWindow.restoreLabel")}</strong><small id="progress-window-restore-hint">{t(savedEnabled ? "progressWindow.restoreHint" : "progressWindow.saveBeforeRestore")}</small></div>
        <button type="button" className="secondary" disabled={busy || !savedEnabled || value.progress_window_enabled === false}
          aria-describedby="progress-window-restore-hint" title={t(savedEnabled ? "progressWindow.restoreHint" : "progressWindow.saveBeforeRestore")}
          onClick={() => { void restore(); }}><ExternalLink size={13} /> {t(busy ? "common.loading" : "progressWindow.restore")}</button>
      </div>
    </div>
    <p className="settings-note">{t("progressWindow.settingsHint")}</p>
  </SettingsGroup>;
}
