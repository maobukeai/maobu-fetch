import { useEffect, useState } from "react";
import { api } from "../../api";
import { t } from "../../i18n";
import type { DownloadTask, ProxyAuth } from "../../types";
import { SettingRow } from "../common/FormComponents";
import { ProxyTestButton } from "../settings/MediaSettingsGroup";

export function TaskProxySection({
  task,
  notify,
}: {
  task: DownloadTask;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [editing, setEditing] = useState(false);
  const [mode, setMode] = useState<"global" | "disable" | "custom">(
    task.proxy_override == null
      ? "global"
      : task.proxy_override === ""
      ? "disable"
      : "custom"
  );
  const [customUrl, setCustomUrl] = useState(task.proxy_override ?? "");
  const [username, setUsername] = useState(task.proxy_auth?.username ?? "");
  const [password, setPassword] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setMode(
      task.proxy_override == null
        ? "global"
        : task.proxy_override === ""
        ? "disable"
        : "custom"
    );
    setCustomUrl(task.proxy_override ?? "");
    setUsername(task.proxy_auth?.username ?? "");
    setPassword("");
    setEditing(false);
  }, [task.id, task.proxy_override, task.proxy_auth]);

  const summary = () => {
    if (task.proxy_override == null) return "使用全局代理设置";
    if (task.proxy_override === "") return "不使用代理";
    return `手动代理：${task.proxy_override}`;
  };

  const startEdit = () => {
    setMode(
      task.proxy_override == null
        ? "global"
        : task.proxy_override === ""
        ? "disable"
        : "custom"
    );
    setCustomUrl(
      task.proxy_override && task.proxy_override !== ""
        ? task.proxy_override
        : ""
    );
    setUsername(task.proxy_auth?.username ?? "");
    setPassword("");
    setEditing(true);
  };

  const save = async () => {
    setSaving(true);
    try {
      let override: string | null;
      if (mode === "global") {
        override = null;
      } else if (mode === "disable") {
        override = "";
      } else {
        const trimmed = customUrl.trim();
        if (!trimmed) {
          notify("自定义代理地址不能为空", "error");
          setSaving(false);
          return;
        }
        override = trimmed;
      }
      const auth: ProxyAuth | null =
        mode === "custom" && username.trim()
          ? { username: username.trim(), password }
          : null;
      await api.updateTaskProxy(task.id, override, auth);
      notify("任务代理设置已保存");
      setEditing(false);
      setPassword("");
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const clearOverride = async () => {
    setSaving(true);
    try {
      await api.updateTaskProxy(task.id, null, null);
      notify("已恢复使用全局代理设置");
      setEditing(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const hasOverride = task.proxy_override != null;

  return (
    <div className="task-retry-policy-section">
      <div className="task-retry-policy-header">
        <strong>代理覆盖</strong>
        <span className="task-retry-policy-summary">
          {editing ? "✏️ 正在编辑代理覆盖设置" : summary()}
        </span>
      </div>
      {!editing && (
        <div className="task-retry-policy-actions">
          <button onClick={startEdit} disabled={saving}>
            编辑代理
          </button>
          {hasOverride && (
            <button onClick={() => void clearOverride()} disabled={saving}>
              恢复全局默认
            </button>
          )}
        </div>
      )}
      {editing && (
        <div className="task-retry-policy-editor">
          <div className="settings-group-content">
            <SettingRow label={t("settings.netProxyMode")}>
              <div className="fluent-segmented-control settings-segmented">
                <button
                  type="button"
                  disabled={saving}
                  className={mode === "global" ? "active" : ""}
                  onClick={() => setMode("global")}
                >
                  {t("settings.netProxyModeGlobal")}
                </button>
                <button
                  type="button"
                  disabled={saving}
                  className={mode === "disable" ? "active" : ""}
                  onClick={() => setMode("disable")}
                >
                  {t("settings.netProxyModeNone")}
                </button>
                <button
                  type="button"
                  disabled={saving}
                  className={mode === "custom" ? "active" : ""}
                  onClick={() => setMode("custom")}
                >
                  {t("settings.netProxyModeManual")}
                </button>
              </div>
            </SettingRow>
            {mode === "custom" && (
              <>
                <SettingRow label={t("settings.netProxyAddressLabel")}>
                  <input
                    value={customUrl}
                    onChange={(e) => setCustomUrl(e.target.value)}
                    placeholder={t("settings.netProxyAddressPlaceholder")}
                    disabled={saving}
                  />
                </SettingRow>
                <SettingRow label={t("settings.netProxyUsername")}>
                  <input
                    value={username}
                    onChange={(e) => setUsername(e.target.value)}
                    placeholder={t("settings.netProxyUsernamePlaceholder")}
                    disabled={saving}
                  />
                </SettingRow>
                <SettingRow label={t("settings.netProxyPassword")}>
                  <input
                    type="password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    placeholder={t("settings.netProxyPasswordPlaceholder")}
                    disabled={saving}
                  />
                </SettingRow>
                <SettingRow label={t("settings.netTestConnectivity")}>
                  <ProxyTestButton
                    proxyUrl={customUrl}
                    auth={username || password ? { username, password } : null}
                    notify={notify}
                    disabled={saving}
                  />
                </SettingRow>
              </>
            )}
          </div>
          <div className="task-retry-policy-actions">
            <button
              className="primary"
              onClick={() => void save()}
              disabled={saving}
            >
              {saving ? "保存中…" : "保存"}
            </button>
            <button
              onClick={() => {
                setEditing(false);
                setPassword("");
              }}
              disabled={saving}
            >
              取消
            </button>
            {hasOverride && (
              <button
                onClick={() => void clearOverride()}
                disabled={saving}
              >
                清除覆盖
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
