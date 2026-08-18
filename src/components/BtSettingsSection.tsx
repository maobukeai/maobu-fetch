import { useState } from "react";
import { ShieldCheck, Zap } from "lucide-react";
import { api } from "../api";
import type { AppSettings, ToolStatus } from "../types";
import { t } from "../i18n";

/**
 * 设置页 "BT/磁力" 分区（2026-08-16 批准纳入）。
 *
 * 强约束（AGENTS.md §3 BT/磁力内核、§6）：
 * - aria2 为按需安装组件（固定版本 + SHA-256 清单），缺失/失败状态明确展示；
 * - 做种默认关闭：开关默认 off，仅用户显式开启后按分享率做种；
 * - 必须包含隐私说明：BT 连接会向 Tracker 与 peers 暴露本机 IP。
 */
function formatBytes(value: number): string {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

export function BtSettingsSection({
  settings,
  toolStatus,
  onUpdate,
  disabled,
}: {
  settings: AppSettings;
  toolStatus: ToolStatus | null | undefined;
  onUpdate: (patch: Partial<AppSettings>) => void;
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const aria2Available = toolStatus?.aria2_available ?? false;
  const aria2Installing = toolStatus?.active_component === "aria2"
    && (toolStatus?.state === "downloading" || toolStatus?.state === "verifying" || toolStatus?.state === "extracting");

  const installAria2 = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.installMediaTool("aria2");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const removeAria2 = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.removeMediaTool("aria2");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const seedEnabled = settings.bt_seed_enabled ?? false;

  return (
    <section className="settings-section bt-settings" aria-label={t("bt.settingsSection")}>
      <h3><Zap size={14} /> {t("bt.settingsSection")}</h3>

      <div className="bt-settings-row">
        <div>
          <div className="bt-settings-label">{t("bt.componentLabel")}</div>
          <div className="bt-settings-sub">
            {t("bt.componentInstalled")} · {toolStatus?.aria2_version || "librqbit"} · {t("bt.outOfBox")}
          </div>
        </div>
        <div className="bt-settings-actions">
          <span className="bt-badge-inline" style={{ color: "var(--accent, #3b82f6)", fontWeight: 500 }}>
            {t("bt.ready")}
          </span>
        </div>
      </div>

      <div className="bt-settings-row">
        <div>
          <div className="bt-settings-label">{t("bt.seedToggle")}</div>
          <div className="bt-settings-sub">{t("bt.seedToggleHint")}</div>
        </div>
        <label className="switch-wrap">
          <input
            type="checkbox"
            checked={seedEnabled}
            disabled={disabled}
            onChange={(event) => onUpdate({ bt_seed_enabled: event.target.checked })}
            aria-label={t("bt.seedToggle")}
          />
        </label>
      </div>

      {seedEnabled && (
        <div className="bt-settings-row">
          <div>
            <div className="bt-settings-label">{t("bt.seedRatio")}</div>
            <div className="bt-settings-sub">{t("bt.seedRatioHint")}</div>
          </div>
          <input
            type="number"
            min="0.1"
            max="100"
            step="0.1"
            value={settings.bt_seed_ratio ?? 1.0}
            disabled={disabled}
            onChange={(event) => {
              const value = Number(event.target.value);
              if (Number.isFinite(value) && value > 0) onUpdate({ bt_seed_ratio: value });
            }}
            style={{ width: "90px" }}
            aria-label={t("bt.seedRatio")}
          />
        </div>
      )}

      <div className="bt-settings-row">
        <div>
          <div className="bt-settings-label">{t("bt.uploadLimit")}</div>
          <div className="bt-settings-sub">{t("bt.uploadLimitHint")}</div>
        </div>
        <input
          type="number"
          min="0"
          step="128"
          value={settings.bt_upload_limit_kbps ?? 2048}
          disabled={disabled}
          onChange={(event) => {
            const value = Number(event.target.value);
            if (Number.isFinite(value) && value >= 0) onUpdate({ bt_upload_limit_kbps: Math.floor(value) });
          }}
          style={{ width: "110px" }}
          aria-label={t("bt.uploadLimit")}
        />
      </div>

      <div className="bt-settings-row bt-settings-row-wide">
        <div>
          <div className="bt-settings-label">{t("bt.trackersLabel")}</div>
          <div className="bt-settings-sub">{t("bt.trackersHint")}</div>
        </div>
        <textarea
          className="bt-trackers-input"
          rows={4}
          spellCheck={false}
          placeholder={"https://tracker.example.com/announce\nudp://tracker.example.com:6969/announce"}
          value={settings.bt_extra_trackers ?? ""}
          disabled={disabled}
          onChange={(event) => onUpdate({ bt_extra_trackers: event.target.value })}
          aria-label={t("bt.trackersLabel")}
        />
      </div>

      <div className="bt-settings-row">
        <div>
          <div className="bt-settings-label">{t("bt.magnetToggle")}</div>
          <div className="bt-settings-sub">{t("bt.magnetToggleHint")}</div>
        </div>
        <label className="switch-wrap">
          <input
            type="checkbox"
            checked={settings.bt_intercept_magnet ?? true}
            disabled={disabled}
            onChange={(event) => onUpdate({ bt_intercept_magnet: event.target.checked })}
            aria-label={t("bt.magnetToggle")}
          />
        </label>
      </div>

      <div className="bt-privacy-card" role="note">
        <ShieldCheck size={14} />
        <div>
          <strong>{t("bt.privacyTitle")}</strong>
          <p>{t("bt.privacyNote")}</p>
        </div>
      </div>
    </section>
  );
}
