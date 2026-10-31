import type { RefObject } from "react";
import { AlertCircle } from "lucide-react";
import { t } from "../../i18n";
import { summaryPercent } from "./progress-model";
import type { ProgressWindowSnapshot } from "./progress-api";

export function ProgressSummary({ snapshot, revealed, hasError, buttonRef, onToggle }: {
  snapshot: ProgressWindowSnapshot | null;
  revealed: boolean;
  hasError: boolean;
  buttonRef: RefObject<HTMLButtonElement | null>;
  onToggle: () => void;
}) {
  const percent = summaryPercent(snapshot);
  const count = snapshot?.total_count ?? 0;
  const progressText = percent === null ? t("progressWindow.sizeUnknown") : `${percent.toFixed(1)}%`;
  return <button ref={buttonRef} type="button" className="progress-window-summary"
    aria-expanded={revealed} aria-controls="progress-window-details"
    aria-label={t("progressWindow.compactLabel", { count, progress: progressText })}
    title={t("progressWindow.compactHint")} onClick={onToggle}>
    {hasError ? <AlertCircle size={12} className="progress-window-summary-alert" aria-hidden="true" />
      : count > 1 && <span className="progress-window-summary-count" aria-hidden="true">{count}</span>}
    <span className={`progress-window-summary-track${percent === null ? " is-unknown" : ""}`}
      role="progressbar" aria-label={t("progressWindow.totalProgress")}
      aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined}
      aria-valuetext={progressText}>
      {percent !== null && <span style={{ width: `${percent}%` }} />}
    </span>
    <span className="progress-window-summary-percent" aria-hidden="true">{percent === null ? "—" : `${Math.floor(percent)}%`}</span>
  </button>;
}
