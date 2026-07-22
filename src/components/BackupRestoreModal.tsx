import { useEffect, useState } from "react";
import { open as pickPath, save as savePath } from "@tauri-apps/plugin-dialog";
import { LoaderCircle, X } from "lucide-react";
import { api } from "../api";
import type { RestorePreview } from "../types";

interface BackupRestoreModalProps {
  notify: (msg: string, kind?: any) => void;
  backupOpen: boolean;
  setBackupOpen: (open: boolean) => void;
  restoreOpen: boolean;
  setRestoreOpen: (open: boolean) => void;
}

export function BackupRestoreModal({
  notify,
  backupOpen,
  setBackupOpen,
  restoreOpen,
  setRestoreOpen,
}: BackupRestoreModalProps) {
  const [backupIncludeAuth, setBackupIncludeAuth] = useState(false);
  const [backupPassword, setBackupPassword] = useState("");
  const [backupBusy, setBackupBusy] = useState(false);

  const [restorePath, setRestorePath] = useState<string | null>(null);
  const [restorePasswordOpen, setRestorePasswordOpen] = useState(false);
  const [restorePassword, setRestorePassword] = useState("");
  const [restorePreview, setRestorePreview] = useState<RestorePreview | null>(null);
  const [restoreBusy, setRestoreBusy] = useState(false);

  const startRestore = async () => {
    try {
      const path = await pickPath({
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      setRestorePath(path);
      setRestorePassword("");
      // 先尝试无密码预览；若文件已加密，后端返回"备份文件已加密，请输入密码"，前端弹密码框。
      const preview = await api.backupPreview(path, null);
      setRestorePreview(preview);
    } catch (error) {
      const message = String(error);
      // P2#7 修复：使用局部变量 path 替代还未更新的 React state restorePath
      if (message.includes("加密")) {
        setRestorePasswordOpen(true);
      } else {
        notify(message, "error");
        setRestorePath(null);
      }
    }
  };

  useEffect(() => {
    if (!restoreOpen) return;
    setRestoreOpen(false);
    void startRestore();
  }, [restoreOpen]);

  const confirmBackup = async () => {
    if (backupIncludeAuth && !backupPassword) {
      notify("包含认证信息的备份必须设置密码", "error");
      return;
    }
    setBackupBusy(true);
    try {
      const path = await savePath({
        defaultPath: "maobu-backup.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) {
        setBackupOpen(false);
        return;
      }
      await api.backupExport(path, backupIncludeAuth, backupIncludeAuth ? backupPassword : null);
      notify(backupIncludeAuth ? "已创建加密备份" : "已创建完整备份");
      setBackupOpen(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setBackupBusy(false);
    }
  };

  const submitRestorePassword = async () => {
    if (!restorePath) return;
    setRestoreBusy(true);
    try {
      const preview = await api.backupPreview(restorePath, restorePassword || null);
      setRestorePreview(preview);
      setRestorePasswordOpen(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setRestoreBusy(false);
    }
  };

  const confirmRestore = async () => {
    if (!restorePath) return;
    setRestoreBusy(true);
    try {
      const stats = await api.backupRestore(restorePath, restorePassword || null);
      notify(
        `恢复完成：新增 ${stats.added_tasks} 个任务，跳过 ${stats.skipped_tasks} 个，应用 ${stats.rules_applied} 条规则${
          stats.settings_replaced ? "，设置已替换" : ""
        }`
      );
      setRestorePreview(null);
      setRestorePath(null);
      setRestorePassword("");
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setRestoreBusy(false);
    }
  };

  const cancelRestore = () => {
    setRestorePreview(null);
    setRestorePath(null);
    setRestorePassword("");
    setRestorePasswordOpen(false);
  };

  return (
    <>
      {backupOpen && (
        <div className="fluent-dialog-overlay">
          <div className="fluent-dialog-surface">
            <div className="fluent-dialog-header">
              <h2>创建完整备份</h2>
              <button onClick={() => setBackupOpen(false)}>
                <X size={16} />
              </button>
            </div>
            <div className="fluent-dialog-body">
              <p>导出下载记录、分类规则、文件名清理规则、平台命名模板、下载预设与全局设置。</p>
              <label className="fluent-checkbox-row">
                <input
                  type="checkbox"
                  checked={backupIncludeAuth}
                  onChange={(e) => setBackupIncludeAuth(e.target.checked)}
                />
                <span>包含认证信息（Cookie / Authorization / 代理密码）</span>
              </label>
              {backupIncludeAuth && (
                <div className="fluent-field">
                  <label>加密密码</label>
                  <input
                    type="password"
                    value={backupPassword}
                    onChange={(e) => setBackupPassword(e.target.value)}
                    placeholder="包含认证信息的备份必须设置密码"
                  />
                </div>
              )}
            </div>
            <div className="fluent-dialog-actions">
              <button
                className="fluent-button primary"
                disabled={backupBusy}
                onClick={() => void confirmBackup()}
              >
                {backupBusy ? <LoaderCircle className="spin" size={14} /> : "导出备份"}
              </button>
              <button className="fluent-button" onClick={() => setBackupOpen(false)}>
                取消
              </button>
            </div>
          </div>
        </div>
      )}

      {restorePasswordOpen && (
        <div className="fluent-dialog-overlay">
          <div className="fluent-dialog-surface">
            <div className="fluent-dialog-header">
              <h2>输入备份解密密码</h2>
              <button onClick={cancelRestore}>
                <X size={16} />
              </button>
            </div>
            <div className="fluent-dialog-body">
              <p>该备份文件已包含加密的认证信息，请输入导出时设置的解密密码。</p>
              <div className="fluent-field">
                <label>密码</label>
                <input
                  type="password"
                  value={restorePassword}
                  onChange={(e) => setRestorePassword(e.target.value)}
                  placeholder="输入备份密码"
                />
              </div>
            </div>
            <div className="fluent-dialog-actions">
              <button
                className="fluent-button primary"
                disabled={restoreBusy || !restorePassword}
                onClick={() => void submitRestorePassword()}
              >
                {restoreBusy ? <LoaderCircle className="spin" size={14} /> : "确认"}
              </button>
              <button className="fluent-button" onClick={cancelRestore}>
                取消
              </button>
            </div>
          </div>
        </div>
      )}

      {restorePreview && (
        <div className="fluent-dialog-overlay">
          <div className="fluent-dialog-surface">
            <div className="fluent-dialog-header">
              <h2>恢复备份预览</h2>
              <button onClick={cancelRestore}>
                <X size={16} />
              </button>
            </div>
            <div className="fluent-dialog-body">
              <p>确认将以下数据应用到当前应用程序：</p>
              <ul>
                <li>新增任务：{restorePreview.new_tasks} 个</li>
                <li>已有重复任务（跳过）：{restorePreview.duplicate_tasks} 个</li>
                <li>新增分类规则：{restorePreview.new_category_rules} 条（覆盖 {restorePreview.override_category_rules} 条）</li>
                <li>新增文件名清理规则：{restorePreview.new_filename_cleanup_rules} 条（覆盖 {restorePreview.override_filename_cleanup_rules} 条）</li>
                <li>新增下载预设：{restorePreview.new_presets} 条（覆盖 {restorePreview.override_presets} 条）</li>
                <li>新增 URL 历史：{restorePreview.new_url_history} 条</li>
                <li>
                  全局设置：
                  {!restorePreview.settings_diff.identical ? "将替换当前设置" : "与当前设置相同"}
                </li>
              </ul>
            </div>
            <div className="fluent-dialog-actions">
              <button
                className="fluent-button primary"
                disabled={restoreBusy}
                onClick={() => void confirmRestore()}
              >
                {restoreBusy ? <LoaderCircle className="spin" size={14} /> : "确认恢复"}
              </button>
              <button className="fluent-button" onClick={cancelRestore}>
                取消
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
