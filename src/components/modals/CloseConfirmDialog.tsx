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
    <Modal title={t("dialogs.closeTitle")} onClose={onClose} style={{ width: "380px" }}>
      <div className="new-task-form" style={{ gap: "16px", padding: "4px 0 0" }}>
        <p style={{ margin: 0, fontSize: "12px", color: "var(--text)", lineHeight: 1.5 }}>
          {t("dialogs.closeQuestion")}
        </p>

        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginTop: "8px",
            width: "100%",
            gap: "12px",
          }}
        >
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: "6px",
              fontSize: "11px",
              color: "var(--muted)",
              cursor: "pointer",
              userSelect: "none",
            }}
          >
            <input
              type="checkbox"
              checked={remember}
              onChange={(e) => setRemember(e.target.checked)}
              style={{
                width: "13px",
                height: "13px",
                accentColor: "var(--accent)",
              }}
            />
            <span>{t("dialogs.rememberChoice")}</span>
          </label>

          <div style={{ display: "flex", gap: "8px" }}>
            <button
              className="dialog-actions-btn"
              style={{
                height: "28px",
                padding: "0 12px",
                borderRadius: "6px",
                border: "1px solid var(--border-strong)",
                background: "var(--control)",
                color: "var(--text)",
                fontSize: "11px",
                fontWeight: 500,
                cursor: "pointer",
              }}
              onClick={() => onConfirm("exit", remember)}
            >
              {t("dialogs.closeExit")}
            </button>
            <button
              className="dialog-actions-btn primary"
              style={{
                height: "28px",
                padding: "0 12px",
                borderRadius: "6px",
                border: "none",
                background: "var(--accent)",
                color: "white",
                fontSize: "11px",
                fontWeight: 500,
                cursor: "pointer",
              }}
              onClick={() => onConfirm("tray", remember)}
            >
              {t("dialogs.closeMinimize")}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
