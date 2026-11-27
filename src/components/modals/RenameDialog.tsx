import { useEffect, useRef, useState } from "react";
import { AlertCircle, FileEdit } from "lucide-react";
import { api } from "../../api";
import { t, useLocale } from "../../i18n";
import type { DownloadTask } from "../../types";
import { Modal } from "../common/Modal";
import {
  validateRenameFilename,
  type RenameValidationError,
} from "./light-modals-model";

const RENAME_ERROR_KEY: Record<RenameValidationError, string> = {
  empty: "dialogs.renamePlaceholder",
  "invalid-chars": "dialogs.renameInvalidChars",
  "path-traversal": "dialogs.renamePathTraversal",
  "too-long": "dialogs.renameTooLong",
};

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
    if (trimmed === task.file_name) {
      onClose();
      return;
    }
    const validationError = validateRenameFilename(value);
    if (validationError) {
      setError(t(RENAME_ERROR_KEY[validationError]));
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
      dialogClassName="lm-dialog"
    >
      <div className="lm-body">
        <div className="lm-file-pill">
          <FileEdit size={13} aria-hidden="true" />
          <span className="lm-file-pill-prefix">
            {t("dialogs.renameOriginal")}：
          </span>
          <span className="lm-file-pill-name" title={task.file_name}>
            {task.file_name}
          </span>
        </div>

        <input
          ref={inputRef}
          className="lm-input"
          aria-label={t("dialogs.renamePlaceholder")}
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setError(undefined);
          }}
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
          className="primary"
          onClick={() => void submit()}
          disabled={busy || !value.trim()}
        >
          {t("common.rename")}
        </button>
      </div>
    </Modal>
  );
}
