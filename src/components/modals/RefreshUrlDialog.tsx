import { useEffect, useRef, useState } from "react";
import { Clipboard, ExternalLink, RefreshCw } from "lucide-react";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "../../api";
import { t, useLocale } from "../../i18n";
import type { DownloadTask } from "../../types";
import { Modal } from "../common/Modal";

export function RefreshUrlDialog({
  task,
  onClose,
  onRefreshed,
  notify,
}: {
  task: DownloadTask;
  onClose: () => void;
  onRefreshed: (updatedTask: DownloadTask) => void;
  notify?: (msg: string, type?: "ok" | "error") => void;
}) {
  useLocale();
  const [newUrl, setNewUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
    // 尝试读取剪贴板中是否有可用链接
    void readText()
      .then((clip) => {
        if (clip) {
          const trimmed = clip.trim();
          if (
            (trimmed.startsWith("http://") ||
              trimmed.startsWith("https://") ||
              trimmed.startsWith("magnet:?")) &&
            trimmed !== task.url
          ) {
            setNewUrl(trimmed);
          }
        }
      })
      .catch(() => {});
  }, [task.url]);

  const handlePasteClipboard = async () => {
    try {
      const clip = await readText();
      if (clip) {
        setNewUrl(clip.trim());
      }
    } catch {
      notify?.(t("common.copyFailed") || "读取剪贴板失败", "error");
    }
  };

  const handleOpenSource = () => {
    if (task.url.startsWith("http://") || task.url.startsWith("https://")) {
      void api.openExternalUrl(task.url);
    }
  };

  const submit = async (andResume = true) => {
    const trimmed = newUrl.trim();
    if (!trimmed) {
      setError(t("dialogs.urlRequired") || "请输入新的下载链接");
      return;
    }
    if (
      !trimmed.startsWith("http://") &&
      !trimmed.startsWith("https://") &&
      !trimmed.startsWith("magnet:?")
    ) {
      setError(t("dialogs.invalidUrlProtocol") || "仅支持 http://、https:// 或 magnet:? 链接");
      return;
    }

    setBusy(true);
    setError(undefined);
    try {
      const updated = await api.refreshTaskUrl(task.id, trimmed);
      if (andResume) {
        try {
          await api.action(task.id, "resume");
        } catch {
          // 若启动失败不影响链接更新完成
        }
      }
      notify?.(t("dialogs.urlRefreshed") || "下载链接已更新", "ok");
      onRefreshed(updated);
      onClose();
    } catch (err: unknown) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("dialogs.refreshUrlTitle") || "刷新下载链接"}
      onClose={onClose}
      style={{ maxWidth: "540px" }}
    >
      <div className="dialog-content" style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
        <div style={{ fontSize: "12px", color: "var(--muted)", lineHeight: 1.5 }}>
          {t("dialogs.refreshUrlDesc") || "针对过期或失效的临时直链（如网盘、CDN），更新地址后将保留已下载分片并原地续传。"}
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
          <label style={{ fontSize: "12px", fontWeight: 600, color: "var(--fg)" }}>
            {t("table.fileName") || "文件名"}
          </label>
          <div
            style={{
              padding: "7px 10px",
              background: "var(--bg-subtle)",
              borderRadius: "6px",
              fontSize: "12px",
              color: "var(--fg)",
              wordBreak: "break-all",
            }}
          >
            {task.file_name}
          </div>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <label style={{ fontSize: "12px", fontWeight: 600, color: "var(--fg)" }}>
              {t("dialogs.originalUrl") || "原下载链接"}
            </label>
            {task.url.startsWith("http") && (
              <button
                type="button"
                className="ghost-button"
                style={{ fontSize: "11px", padding: "2px 6px", display: "inline-flex", alignItems: "center", gap: "4px" }}
                onClick={handleOpenSource}
                title={t("dialogs.openOriginalInBrowser") || "在浏览器中打开原网页"}
              >
                <ExternalLink size={11} />
                {t("dialogs.openInBrowser") || "打开原链接"}
              </button>
            )}
          </div>
          <div
            style={{
              padding: "6px 10px",
              background: "var(--bg-subtle)",
              borderRadius: "6px",
              fontSize: "11px",
              color: "var(--muted)",
              wordBreak: "break-all",
              maxHeight: "60px",
              overflowY: "auto",
            }}
          >
            {task.url}
          </div>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <label style={{ fontSize: "12px", fontWeight: 600, color: "var(--fg)" }}>
              {t("dialogs.newUrl") || "新下载链接"}
            </label>
            <button
              type="button"
              className="ghost-button"
              style={{ fontSize: "11px", padding: "2px 6px", display: "inline-flex", alignItems: "center", gap: "4px" }}
              onClick={handlePasteClipboard}
            >
              <Clipboard size={11} />
              {t("common.paste") || "粘贴剪贴板"}
            </button>
          </div>
          <input
            ref={inputRef}
            className="input-text"
            type="text"
            placeholder="https://..."
            value={newUrl}
            onChange={(e) => setNewUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !busy) {
                void submit(true);
              }
            }}
            disabled={busy}
            style={{ width: "100%", fontSize: "12px" }}
          />
        </div>

        {error && (
          <div style={{ fontSize: "12px", color: "var(--danger)", marginTop: "2px" }}>
            {error}
          </div>
        )}

        <div
          className="dialog-actions"
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: "8px",
            marginTop: "8px",
          }}
        >
          <button className="button-secondary" type="button" onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </button>
          <button
            className="button-secondary"
            type="button"
            onClick={() => void submit(false)}
            disabled={busy || !newUrl.trim()}
          >
            {t("dialogs.updateOnly") || "仅更新链接"}
          </button>
          <button
            className="button-primary"
            type="button"
            onClick={() => void submit(true)}
            disabled={busy || !newUrl.trim()}
            style={{ display: "inline-flex", alignItems: "center", gap: "6px" }}
          >
            <RefreshCw size={13} className={busy ? "spin" : ""} />
            {busy ? t("common.saving") : t("dialogs.updateAndResume") || "更新并继续下载"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
