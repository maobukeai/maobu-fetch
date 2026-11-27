import { useEffect, useRef, useState } from "react";
import { AlertCircle, Gauge } from "lucide-react";
import { t, useLocale } from "../../i18n";
import type { DownloadTask } from "../../types";
import { Modal } from "../common/Modal";
import {
  SPEED_LIMIT_PRESETS,
  formatSpeedLimitBadge,
} from "./light-modals-model";

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
    const trimmed = value.trim();
    const limit = trimmed === "" ? Number.NaN : Number(trimmed);
    if (!Number.isFinite(limit) || limit < 0) {
      setError(t("dialogs.speedLimitInvalid"));
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

  const parsedNumeric = value.trim() === "" ? Number.NaN : Number(value);
  const liveBadge =
    Number.isFinite(parsedNumeric) && parsedNumeric >= 0
      ? formatSpeedLimitBadge(parsedNumeric * 1024, t("details.noSpeedLimit"))
      : formatSpeedLimitBadge(
          task.per_task_speed_limit,
          t("details.noSpeedLimit")
        );

  return (
    <Modal
      title={t("dialogs.speedLimitTitle")}
      onClose={onClose}
      dialogClassName="lm-dialog"
      headerAction={<span className="lm-eyebrow">{liveBadge}</span>}
    >
      <div className="lm-body">
        <div className="lm-file-pill">
          <Gauge size={13} aria-hidden="true" />
          <span className="lm-file-pill-name" title={task.file_name}>
            {task.file_name}
          </span>
        </div>

        <div className="lm-presets" role="group" aria-label={t("dialogs.speedLimitLabel")}>
          {SPEED_LIMIT_PRESETS.map((preset) => {
            const active =
              Number.isFinite(parsedNumeric) && parsedNumeric === preset.kb;
            const text = preset.labelKey ? t(preset.labelKey) : preset.label;
            return (
              <button
                key={preset.kb}
                type="button"
                className="lm-preset-chip"
                data-active={active ? "true" : "false"}
                aria-pressed={active}
                disabled={busy}
                onClick={() => {
                  setValue(String(preset.kb));
                  setError(undefined);
                  inputRef.current?.focus();
                }}
              >
                {text}
              </button>
            );
          })}
        </div>

        <div className="lm-input-unit-wrap">
          <input
            ref={inputRef}
            className="lm-input tabular"
            aria-label={t("dialogs.speedLimitLabel")}
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
          <span className="lm-input-unit">KB/s</span>
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
          className="primary"
          onClick={() => void submit()}
          disabled={busy}
        >
          {t("dialogs.speedLimitApply")}
        </button>
      </div>
    </Modal>
  );
}
