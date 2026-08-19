import { useEffect, useRef, useState } from "react";
import { t, useLocale } from "../../i18n";
import type { DownloadTask } from "../../types";
import { Modal } from "../common/Modal";

export function SpeedLimitDialog({
  task,
  onClose,
  onConfirm,
}: {
  task: DownloadTask;
  onClose: () => void;
  onConfirm: (limitKb: number) => Promise<void>;
}) {
  useLocale();
  const currentLimit = Math.round(task.per_task_speed_limit / 1024);
  const [value, setValue] = useState(String(currentLimit));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const submit = async () => {
    const limit = Number(value);
    if (!Number.isFinite(limit) || limit < 0) {
      setError("请输入不小于 0 的有效数字");
      return;
    }
    setBusy(true);
    try {
      await onConfirm(limit);
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title="任务限速" onClose={onClose} style={{ width: "420px" }}>
      <div className="delete-task-dialog">
        <p className="delete-task-message">
          任务限速：<strong title={task.file_name}>{task.file_name}</strong>
        </p>
        <label className="form-field">
          <span>限速值 (KB/s，0 表示不限速)</span>
          <input
            ref={inputRef}
            type="number"
            min="0"
            step="1"
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
            placeholder="0"
            disabled={busy}
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
            disabled={busy}
          >
            {t("common.ok")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
