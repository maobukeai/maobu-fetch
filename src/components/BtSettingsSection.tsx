import { useState } from "react";
import { ShieldCheck, Zap } from "lucide-react";
import { api } from "../api";
import type { AppSettings, ToolStatus } from "../types";
import { t } from "../i18n";
import { SettingRow, SettingsGroup, Toggle } from "./common/FormComponents";
import { Select } from "./Select";

const TRACKER_PRESETS = [
  {
    value: "https://cf.trackerslist.com/best.txt",
    label: "🔥 国内 CDN 镜像 · 精选高速（推荐直连）",
    key: "bt.presetCdnBest",
  },
  {
    value: "https://cf.trackerslist.com/all.txt",
    label: "⚡ 国内 CDN 镜像 · 全量有效节点",
    key: "bt.presetCdnAll",
  },
  {
    value: "https://ghfast.top/https://raw.githubusercontent.com/ngosang/trackerslist/master/trackers_best.txt",
    label: "🚀 GitHub 镜像加速（ghfast）",
    key: "bt.presetGhfast",
  },
  {
    value: "https://raw.githubusercontent.com/ngosang/trackerslist/master/trackers_best.txt",
    label: "🌐 GitHub 官方 · 精选列表（ngosang）",
    key: "bt.presetGithubBest",
  },
  {
    value: "https://raw.githubusercontent.com/ngosang/trackerslist/master/trackers_all.txt",
    label: "📦 GitHub 官方 · 全量列表（ngosang）",
    key: "bt.presetGithubAll",
  },
];

/**
 * 设置页 "BT/磁力" 分区（与全局 Fluent 风格统一）。
 *
 * 遵循 Windows 11 Fluent 规范：
 * - 统一使用 SettingsGroup / SettingRow / Toggle / Select 组件
 * - 统一输入框、数字框与操作按钮的圆角、高度与焦点样式
 * - Tracker 订阅支持两行式大宽度布局与多预设一键切换
 */
export function BtSettingsSection({
  settings,
  toolStatus,
  onUpdate,
  disabled,
  notify,
}: {
  settings: AppSettings;
  toolStatus: ToolStatus | null | undefined;
  onUpdate: (patch: Partial<AppSettings>) => void;
  disabled?: boolean;
  notify?: (text: string, kind?: "ok" | "error") => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const seedEnabled = settings.bt_seed_enabled ?? false;

  const currentUrl =
    settings.bt_tracker_subscribe_url ?? TRACKER_PRESETS[0].value;
  const isPreset = TRACKER_PRESETS.some((p) => p.value === currentUrl);
  const selectedPresetValue = isPreset ? currentUrl : "custom";

  const validTrackerCount = (settings.bt_extra_trackers ?? "")
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith("#")).length;

  const presetOptions = [
    ...TRACKER_PRESETS.map((preset) => ({
      value: preset.value,
      label: t(preset.key) || preset.label,
    })),
    {
      value: "custom",
      label: t("bt.customSource") || "✏️ 自定义订阅源",
    },
  ];

  const handleUpdateTrackers = async () => {
    setBusy(true);
    setError(null);
    try {
      const count = await api.updateBtTrackers(currentUrl);
      const latest = await api.settings();
      onUpdate({ bt_extra_trackers: latest.bt_extra_trackers });
      notify?.(
        t("bt.trackersUpdated", { count }) ||
          `已成功更新并同步 ${count} 个 Tracker 服务器`,
        "ok"
      );
    } catch (e) {
      const msg = String(e);
      setError(msg);
      notify?.(msg, "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <SettingsGroup title={t("bt.componentGroup") || "内核与基础设置"}>
        <div className="settings-group-content">
          <SettingRow
            label={t("bt.componentLabel")}
            sub={`${t("bt.componentInstalled")} · ${toolStatus?.aria2_version || "librqbit"} · ${t("bt.outOfBox")}`}
          >
            <span
              style={{
                color: "var(--accent, #0078d4)",
                fontWeight: 600,
                fontSize: "12px",
                padding: "2px 8px",
                borderRadius: "4px",
                background: "var(--surface-selected, rgba(0, 120, 212, 0.08))",
              }}
            >
              {t("bt.ready")}
            </span>
          </SettingRow>

          <Toggle
            label={t("bt.magnetToggle")}
            sub={t("bt.magnetToggleHint")}
            checked={settings.bt_intercept_magnet ?? true}
            onChange={(checked) => onUpdate({ bt_intercept_magnet: checked })}
          />
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("bt.seedGroup") || "做种与上传策略"}>
        <div className="settings-group-content">
          <Toggle
            label={t("bt.seedToggle")}
            sub={t("bt.seedToggleHint")}
            checked={seedEnabled}
            onChange={(checked) => onUpdate({ bt_seed_enabled: checked })}
          />

          {seedEnabled && (
            <SettingRow
              label={t("bt.seedRatio")}
              sub={t("bt.seedRatioHint")}
            >
              <input
                type="number"
                min="0.1"
                max="100"
                step="0.1"
                value={settings.bt_seed_ratio ?? 1.0}
                disabled={disabled}
                onChange={(event) => {
                  const value = Number(event.target.value);
                  if (Number.isFinite(value) && value > 0)
                    onUpdate({ bt_seed_ratio: value });
                }}
                style={{ width: "90px" }}
                aria-label={t("bt.seedRatio")}
              />
            </SettingRow>
          )}

          <SettingRow
            label={t("bt.uploadLimit")}
            sub={t("bt.uploadLimitHint")}
          >
            <input
              type="number"
              min="0"
              step="128"
              value={settings.bt_upload_limit_kbps ?? 2048}
              disabled={disabled}
              onChange={(event) => {
                const value = Number(event.target.value);
                if (Number.isFinite(value) && value >= 0)
                  onUpdate({ bt_upload_limit_kbps: Math.floor(value) });
              }}
              style={{ width: "100px" }}
              aria-label={t("bt.uploadLimit")}
            />
          </SettingRow>
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("bt.trackersGroup") || "Tracker 订阅与加速"}>
        <div
          className="settings-group-content"
          style={{
            padding: "16px 20px",
            display: "flex",
            flexDirection: "column",
            gap: "12px",
          }}
        >
          {/* 第一行：左侧标题说明 + 右侧使用统一的 Fluent Select 组件 */}
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "16px",
              flexWrap: "wrap",
            }}
          >
            <div style={{ flex: 1, minWidth: "200px" }}>
              <strong
                style={{
                  display: "block",
                  fontSize: "13px",
                  fontWeight: 500,
                  color: "var(--text)",
                }}
              >
                {t("bt.trackerSubscribeLabel") || "Tracker 订阅源与更新"}
              </strong>
              <small
                style={{
                  display: "block",
                  marginTop: "4px",
                  color: "var(--muted)",
                  fontSize: "11px",
                  lineHeight: "1.4",
                }}
              >
                {t("bt.trackerSubscribeHint") ||
                  "配置远程 Tracker 订阅源（如 GitHub 高可用列表），一键更新以获得更快的 BT 磁力寻道速度。"}
              </small>
            </div>

            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                flexShrink: 0,
                width: "280px",
              }}
            >
              <Select
                value={selectedPresetValue}
                onChange={(nextVal) => {
                  if (nextVal !== "custom") {
                    onUpdate({ bt_tracker_subscribe_url: String(nextVal) });
                  }
                }}
                options={presetOptions}
                ariaLabel={t("bt.presetSources") || "推荐预设"}
                style={{ width: "100%", height: "31px", fontSize: "11.5px" }}
              />
            </div>
          </div>

          {/* 第二行：满宽输入框 + 立即更新按钮 */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "8px",
              width: "100%",
            }}
          >
            <input
              type="text"
              placeholder="https://..."
              value={currentUrl}
              disabled={disabled || busy}
              onChange={(e) =>
                onUpdate({ bt_tracker_subscribe_url: e.target.value })
              }
              style={{
                flex: 1,
                minWidth: 0,
                height: "31px",
                padding: "0 10px",
                borderRadius: "6px",
                border: "1px solid var(--border-strong)",
                background: "var(--control)",
                color: "var(--text)",
                fontSize: "11.5px",
              }}
            />
            <button
              type="button"
              className="input-button"
              disabled={disabled || busy}
              onClick={handleUpdateTrackers}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "5px",
                height: "31px",
                padding: "0 14px",
                fontSize: "11.5px",
                fontWeight: 500,
                borderRadius: "6px",
                border: "1px solid var(--border-strong)",
                background: "var(--control)",
                color: "var(--text)",
                cursor: "pointer",
                whiteSpace: "nowrap",
                flexShrink: 0,
              }}
            >
              <Zap size={12} />
              {busy
                ? t("common.loading") || "更新中…"
                : t("bt.updateTrackersNow") || "立即更新 Trackers"}
            </button>
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("bt.trackersLabel") || "额外 Tracker 列表"}>
        <div
          className="settings-group-content"
          style={{
            padding: "16px 20px",
            display: "flex",
            flexDirection: "column",
            gap: "10px",
          }}
        >
          <textarea
            className="bt-trackers-textarea"
            rows={6}
            spellCheck={false}
            placeholder={
              "https://tracker.example.com/announce\nudp://tracker.example.com:6969/announce"
            }
            value={settings.bt_extra_trackers ?? ""}
            disabled={disabled}
            onChange={(event) =>
              onUpdate({ bt_extra_trackers: event.target.value })
            }
            aria-label={t("bt.trackersLabel")}
          />
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              fontSize: "11px",
              color: "var(--muted)",
            }}
          >
            <span>
              {validTrackerCount > 0
                ? `已包含 ${validTrackerCount} 个有效 Tracker 节点`
                : "尚未添加额外 Tracker 节点"}
            </span>
            {validTrackerCount > 0 && (
              <button
                type="button"
                className="input-button"
                onClick={() => onUpdate({ bt_extra_trackers: "" })}
                style={{
                  height: "24px",
                  padding: "0 10px",
                  fontSize: "11px",
                  borderRadius: "4px",
                  cursor: "pointer",
                }}
              >
                {t("common.clear") || "清空"}
              </button>
            )}
          </div>
        </div>
        <p className="settings-note">{t("bt.trackersHint")}</p>
      </SettingsGroup>

      {error && (
        <p
          className="settings-error"
          style={{
            color: "var(--danger)",
            fontSize: "11px",
            margin: "8px 0",
          }}
        >
          {error}
        </p>
      )}

      <div
        className="bt-privacy-card"
        role="note"
        style={{ marginTop: "16px" }}
      >
        <ShieldCheck size={14} />
        <div>
          <strong>{t("bt.privacyTitle")}</strong>
          <p>{t("bt.privacyNote")}</p>
        </div>
      </div>
    </>
  );
}
