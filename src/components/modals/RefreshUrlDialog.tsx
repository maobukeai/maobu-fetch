import { useEffect, useRef, useState } from "react";
import { AlertCircle, Clipboard, ExternalLink, Link2, RefreshCw } from "lucide-react";
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
      dialogClassName="lm-dialog"
      headerAction={
        <span className="lm-eyebrow">{t("dialogs.refreshResumeBadge")}</span>
      }
    >
      <div className="lm-body">
        <div className="lm-file-pill">
          <Link2 size={13} aria-hidden="true" />
          <span className="lm-file-pill-name" title={task.file_name}>
            {task.file_name}
          </span>
        </div>

        <div className="lm-field">
          <div className="lm-field-header">
            <span className="lm-field-label">
              {t("dialogs.originalUrl") || "原链接"}
            </span>
            {task.url.startsWith("http") && (
              <button
                type="button"
                className="lm-ghost-btn"
                onClick={handleOpenSource}
                title={
                  t("dialogs.openOriginalInBrowser") || "在浏览器中打开原网页"
                }
              >
                <ExternalLink size={11} />
                {t("dialogs.openInBrowser") || "打开原链接"}
              </button>
            )}
          </div>
          <div className="lm-url-box">{task.url}</div>
        </div>

        <div className="lm-field">
          <div className="lm-field-header">
            <label htmlFor="refresh-url-input" className="lm-field-label">
              {t("dialogs.newUrl") || "新链接"}
            </label>
            <button
              type="button"
              className="lm-ghost-btn"
              onClick={handlePasteClipboard}
            >
              <Clipboard size={11} />
              {t("common.paste") || "粘贴"}
            </button>
          </div>
          <input
            id="refresh-url-input"
            ref={inputRef}
            className="lm-input"
            type="text"
            placeholder="https://..."
            value={newUrl}
            onChange={(e) => {
              setNewUrl(e.target.value);
              setError(undefined);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !busy) {
                void submit(true);
              }
            }}
            disabled={busy}
          />
        </div>

        {error && (
          <div className="lm-error" role="alert">
            <AlertCircle size={13} />
            <span>{error}</span>
          </div>
        )}
      </div>

      <div className="dialog-actions">
        <button type="button" onClick={onClose} disabled={busy}>
          {t("common.cancel")}
        </button>
        <button
          type="button"
          onClick={() => void submit(false)}
          disabled={busy || !newUrl.trim()}
        >
          {t("dialogs.updateOnly") || "仅更新链接"}
        </button>
        <button
          type="button"
          className="primary"
          onClick={() => void submit(true)}
          disabled={busy || !newUrl.trim()}
          style={{ display: "inline-flex", alignItems: "center", gap: "6px" }}
        >
          <RefreshCw size={13} className={busy ? "spin" : ""} />
          {busy
            ? t("common.saving")
            : t("dialogs.updateAndResume") || "更新并继续下载"}
        </button>
      </div>
    </Modal>
  );
}
