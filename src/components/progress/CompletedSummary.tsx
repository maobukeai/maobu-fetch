import type { RefObject } from "react";
import { AlertCircle, CheckCircle2, FileSymlink, FolderOpen, X } from "lucide-react";
import { t } from "../../i18n";
import { completedCount, completedTasks } from "./progress-model";
import { progressApi, type ProgressOperation, type ProgressWindowSnapshot } from "./progress-api";

export function CompletedSummary({ snapshot, revealed, busy, hasError, buttonRef, onReveal, onDismiss, perform }: {
  snapshot: ProgressWindowSnapshot; revealed: boolean; busy: string | null;
  hasError: boolean;
  buttonRef: RefObject<HTMLButtonElement | null>; onReveal: () => void; onDismiss: () => void; perform: ProgressOperation;
}) {
  const latest = completedTasks(snapshot)[0];
  const count = completedCount(snapshot);
  const open = (target: "file" | "folder") => {
    if (latest) void perform(`open:${latest.id}:${target}`, () => progressApi.openCompleted(latest.id, target), "progressWindow.openError");
  };
  return <div className="progress-window-summary progress-window-completed-summary" role="group" aria-label={t("progressWindow.completedCount", { count })}>
    <button ref={buttonRef} type="button" className="progress-window-completed-reveal"
      aria-expanded={revealed} aria-controls="progress-window-details" title={t(hasError ? "progressWindow.openError" : "progressWindow.completedHint")} onClick={onReveal}>
      {hasError ? <AlertCircle size={12} className="progress-window-summary-alert" aria-hidden="true" /> : <CheckCircle2 size={12} aria-hidden="true" />}<span>{t("progressWindow.completedCount", { count })}</span>
    </button>
    <button type="button" className="progress-window-icon progress-window-completed-open" disabled={Boolean(busy) || !latest}
      title={t("progressWindow.openFile")} aria-label={t("progressWindow.openFileOf", { name: latest?.file_name ?? "" })} onClick={() => open("file")}><FileSymlink size={12} /></button>
    <button type="button" className="progress-window-icon progress-window-completed-folder" disabled={Boolean(busy) || !latest}
      title={t("progressWindow.openFolder")} aria-label={t("progressWindow.openFolderOf", { name: latest?.file_name ?? "" })} onClick={() => open("folder")}><FolderOpen size={12} /></button>
    <button type="button" className="progress-window-icon" disabled={Boolean(busy)} title={t("progressWindow.hide")}
      aria-label={t("progressWindow.hide")} onClick={onDismiss}><X size={12} /></button>
  </div>;
}
