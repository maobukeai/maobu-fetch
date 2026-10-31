import { useEffect, useState } from "react";
import { CheckCircle2, FolderOpen, Play, X } from "lucide-react";
import { api } from "../api";
import { t } from "../i18n";
import type { TaskNotificationPayload } from "../types";

interface CompletionToastProps {
  notices: TaskNotificationPayload[];
  durationSeconds: number;
  onDismiss: (taskId: string) => void;
  onView: (taskId: string) => void;
  onError: (message: string) => void;
}

/** 完成提醒依次展示；用户操作、窗口失焦或隐藏时暂停自动关闭。 */
export function CompletionToast({
  notices, durationSeconds, onDismiss, onView, onError,
}: CompletionToastProps) {
  const notice = notices[0];
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [busy, setBusy] = useState(false);
  const [windowActive, setWindowActive] = useState(
    () => !document.hidden && document.hasFocus(),
  );

  useEffect(() => {
    const update = () => setWindowActive(!document.hidden && document.hasFocus());
    document.addEventListener("visibilitychange", update);
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    return () => {
      document.removeEventListener("visibilitychange", update);
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
    };
  }, []);

  useEffect(() => {
    if (!notice || durationSeconds === 0 || hovered || focused || busy || !windowActive) return;
    const timer = window.setTimeout(() => onDismiss(notice.task_id), durationSeconds * 1000);
    return () => window.clearTimeout(timer);
  }, [notice, durationSeconds, hovered, focused, busy, windowActive, onDismiss]);

  if (!notice) return null;

  const open = async (folder: boolean) => {
    setBusy(true);
    try {
      if (folder) await api.openFolder(notice.task_id);
      else await api.openFile(notice.task_id);
      onDismiss(notice.task_id);
    } catch (error) {
      onError(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="toast toast-with-action completion-toast"
      role="status"
      aria-live="polite"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false);
      }}
    >
      <span className="toast-icon"><CheckCircle2 size={16} /></span>
      <div className="toast-body">
        <strong className="completion-toast-title" title={notice.title}>{notice.title}</strong>
        <span className="toast-subtext">{notice.body}</span>
        <div className="toast-actions">
          <button type="button" className="toast-action-btn" disabled={busy} onClick={() => void open(false)}>
            <Play size={11} />{t("contextMenu.openFile")}
          </button>
          <button type="button" className="toast-action-btn toast-action-btn-secondary" disabled={busy} onClick={() => void open(true)}>
            <FolderOpen size={11} />{t("contextMenu.openFolder")}
          </button>
          <button type="button" className="toast-action-btn toast-action-btn-secondary" disabled={busy} onClick={() => {
            onView(notice.task_id);
            onDismiss(notice.task_id);
          }}>
            {t("toasts.viewDetails")}
          </button>
        </div>
        {notices.length > 1 && <span className="toast-subtext">{t("toasts.moreCompletions", { count: notices.length - 1 })}</span>}
      </div>
      <button type="button" className="toast-close-btn" disabled={busy} onClick={() => onDismiss(notice.task_id)} aria-label={t("common.close")}>
        <X size={12} />
      </button>
    </div>
  );
}
