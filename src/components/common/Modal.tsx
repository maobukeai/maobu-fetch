import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";

export const modalEscapeStack: Array<() => void> = [];

export const FOCUSABLE_SELECTOR = [
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "a[href]",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

export function Modal({
  title,
  onClose,
  wide,
  children,
  style,
  headerAction,
  escapeClosable = true,
}: {
  title: string;
  onClose: () => void;
  wide?: boolean;
  children: ReactNode;
  style?: CSSProperties;
  headerAction?: ReactNode;
  escapeClosable?: boolean;
}) {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const escapeClosableRef = useRef(escapeClosable);
  escapeClosableRef.current = escapeClosable;

  useEffect(() => {
    const entry = () => {
      if (escapeClosableRef.current) onCloseRef.current();
    };
    modalEscapeStack.push(entry);

    const handleKeyDown = (event: KeyboardEvent) => {
      if (modalEscapeStack[modalEscapeStack.length - 1] !== entry) return;
      if (event.key === "Escape") {
        event.preventDefault();
        entry();
        return;
      }
      if (event.key === "Tab") {
        const dialog = dialogRef.current;
        if (!dialog) return;
        const focusable = Array.from(
          dialog.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)
        ).filter((el) => el.offsetParent !== null);
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        const active = document.activeElement;
        if (event.shiftKey && (active === first || !dialog.contains(active))) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && (active === last || !dialog.contains(active))) {
          event.preventDefault();
          first.focus();
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown, true);
    const dialog = dialogRef.current;
    if (dialog && !dialog.contains(document.activeElement)) {
      const target =
        dialog.querySelector<HTMLElement>("[autofocus]") ??
        dialog.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      (target ?? dialog).focus();
    }

    return () => {
      const index = modalEscapeStack.indexOf(entry);
      if (index >= 0) modalEscapeStack.splice(index, 1);
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, []);

  return (
    <div className="modal-layer" onMouseDown={onClose}>
      <div
        className="dialog-material"
        ref={dialogRef}
        tabIndex={-1}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <section className={wide ? "dialog wide" : "dialog"} style={style}>
          <div className="settings-title">
            <h2>{title}</h2>
            {headerAction}
          </div>
          {children}
        </section>
      </div>
    </div>
  );
}

export function ConfirmDialog({
  title,
  message,
  confirmLabel,
  cancelLabel,
  danger,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return createPortal(
    <Modal title={title} onClose={onCancel} style={{ width: "400px" }}>
      <div className="confirm-dialog-body">
        <p>{message}</p>
        <div className="confirm-dialog-actions">
          <button
            className="confirm-btn-secondary"
            onClick={onCancel}
            autoFocus={danger}
          >
            {cancelLabel}
          </button>
          <button
            className={danger ? "confirm-btn-danger" : "confirm-btn-primary"}
            onClick={onConfirm}
            autoFocus={!danger}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </Modal>,
    document.body
  );
}
