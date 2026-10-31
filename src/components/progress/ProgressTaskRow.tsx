import { Pause } from "lucide-react";
import { formatBytes } from "../../formatters";
import { t } from "../../i18n";
import { nonNegative, taskPercent } from "./progress-model";
import type { ProgressWindowTask } from "./progress-api";

function stateText(task: ProgressWindowTask): string {
  if (task.fetching_metadata) return t("progressWindow.fetchingMetadata");
  const keys: Record<string, string> = {
    queued: "status.queued", downloading: "status.downloading", verifying: "status.verifying",
    paused: "status.paused", "waiting-network": "progressWindow.waitingNetwork",
  };
  return t(keys[task.status] ?? "progressWindow.processing");
}

export function ProgressTaskRow({ task, disabled, onAction, onOpen }: {
  task: ProgressWindowTask;
  disabled: boolean;
  onAction: () => void;
  onOpen: () => void;
}) {
  const percent = taskPercent(task);
  const downloaded = formatBytes(nonNegative(task.downloaded_bytes));
  const size = percent === null ? t("progressWindow.sizeUnknown") : formatBytes(task.total_bytes);
  const actionText = t("progressWindow.pause");
  const name = task.file_name || t("progressWindow.fetchingMetadata");
  const state = stateText(task);
  const showState = task.status !== "downloading" || task.fetching_metadata || percent === null;
  const percentText = percent === null ? "—" : `${Math.floor(percent)}%`;
  return (
    <li className="progress-window-task" data-task-id={task.id}>
      <div className="progress-window-task-content">
        <div className="progress-window-task-top">
          <button type="button" className="progress-window-task-name" onClick={onOpen} disabled={disabled}
            title={`${name}\n${downloaded} / ${size}`} aria-label={t("progressWindow.viewTask", { name })}>{name}</button>
          <span className={`progress-window-task-state${showState ? " is-status" : ""}`} title={state}>
            {showState ? state : percentText}
          </span>
        </div>
        <div className="progress-window-task-meta">
          <div className={`progress-window-track${percent === null ? " is-unknown" : ""}`}
            role="progressbar" aria-label={t("progressWindow.progressOf", { name })}
            aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined}
            aria-valuetext={percent === null ? `${downloaded} · ${t("progressWindow.sizeUnknown")}` : `${percent.toFixed(1)}%`}>
            {percent !== null && <span style={{ width: `${percent}%` }} />}
          </div>
          {task.status === "downloading" && !task.fetching_metadata &&
            <span>{formatBytes(nonNegative(task.speed_bps))}/s</span>}
          {(task.status !== "downloading" || task.fetching_metadata) &&
            <span className="progress-window-task-status">{percentText}</span>}
        </div>
      </div>
      {task.can_pause && <button type="button" className="progress-window-icon progress-window-task-action"
        onClick={onAction} disabled={disabled} title={actionText} aria-label={`${actionText}：${name}`}>
        <Pause size={11} />
      </button>}
    </li>
  );
}
