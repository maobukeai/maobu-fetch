import { useEffect, useRef, useState } from "react";
import { api } from "../../api";
import { t, useLocale } from "../../i18n";
import type { DownloadTask } from "../../types";
import { Modal } from "../common/Modal";

export function RenameDialog({
  task,
  onClose,
  onRenamed,
}: {
  task: DownloadTask;
  onClose: () => void;
  onRenamed: (newName: string) => void;
}) {
  useLocale();
  const [value, setValue] = useState(task.file_name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
    const dot = task.file_name.lastIndexOf(".");
    inputRef.current?.setSelectionRange(
      0,
      dot > 0 ? dot : task.file_name.length
    );
  }, [task.file_name]);

  const submit = async () => {
    const trimmed = value.trim();
    if (!trimmed) {
      setError(t("dialogs.renamePlaceholder"));
      return;
    }
    if (trimmed === task.file_name) {
      onClose();
      return;
    }
    if (/[<>:"/\\|?*]/.test(trimmed) || /[\x00-\x1f]/.test(trimmed)) {
      setError(`${trimmed}`);
      return;
    }
    if (
      trimmed.includes("..") ||
      trimmed.startsWith("/") ||
      trimmed.startsWith("\\")
    ) {
      setError(`${trimmed}`);
      return;
    }
    if (trimmed.length > 255) {
      setError(`${trimmed}`);
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      await api.rename(task.id, trimmed);
      onRenamed(trimmed);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("dialogs.renameTitle")}
      onClose={onClose}
      style={{ width: "420px" }}
    >
      <div className="delete-task-dialog">
        <p className="delete-task-message">
          {t("dialogs.renameTitle")}：
          <strong title={task.file_name}>{task.file_name}</strong>
        </p>
        <label className="form-field">
          <span>{t("dialogs.renamePlaceholder")}</span>
          <input
            ref={inputRef}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void submit();
              }
              if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
            disabled={busy}
            placeholder={t("dialogs.renamePlaceholder")}
          />
        </label>
        {error && <p className="inline-error">{error}</p>}
        <div className="dialog-actions">
          <button onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </button>
          <button
            className="primary"
            onClick={() => void submit()}
            disabled={busy || !value.trim()}
          >
            {t("common.rename")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
