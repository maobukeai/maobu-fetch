import { useState } from "react";
import { t, useLocale } from "../../i18n";
import { Modal } from "../common/Modal";

export function CloseConfirmDialog({
  onClose,
  onConfirm,
}: {
  onClose: () => void;
  onConfirm: (action: "tray" | "exit", remember: boolean) => void;
}) {
  useLocale();
  const [remember, setRemember] = useState(false);

  return (
    <Modal
      title={t("dialogs.closeTitle")}
      onClose={onClose}
      dialogClassName="lm-dialog lm-dialog-compact"
    >
      <div className="lm-body">
        <p className="lm-lead">{t("dialogs.closeQuestion")}</p>
      </div>

      <div className="lm-close-footer">
        <label className="lm-check">
          <input
            type="checkbox"
            checked={remember}
            onChange={(e) => setRemember(e.target.checked)}
          />
          <span>{t("dialogs.rememberChoice")}</span>
        </label>

        <div className="lm-close-footer-actions">
          <button type="button" onClick={() => onConfirm("exit", remember)}>
            {t("dialogs.closeExit")}
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => onConfirm("tray", remember)}
          >
            {t("dialogs.closeMinimize")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
