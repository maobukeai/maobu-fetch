import { File, FileSymlink, FolderOpen } from "lucide-react";
import { formatBytes } from "../../formatters";
import { t } from "../../i18n";
import { nonNegative } from "./progress-model";
import type { ProgressWindowTask } from "./progress-api";

export function CompletedTaskRow({ task, disabled, showActions = true, onOpen }: {
  task: ProgressWindowTask; disabled: boolean; showActions?: boolean; onOpen: (target: "file" | "folder") => void;
}) {
  const size = nonNegative(task.total_bytes) || nonNegative(task.downloaded_bytes);
  return <li className="progress-window-task progress-window-completed-row" data-completed-id={task.id}>
    <span className="progress-window-file-icon" aria-hidden="true"><File size={16} /></span>
    <div className="progress-window-task-content">
      <button type="button" className="progress-window-task-name" disabled={disabled} onClick={() => onOpen("file")}
        title={task.file_name} aria-label={t("progressWindow.openFileOf", { name: task.file_name })}>{task.file_name}</button>
      <span className="progress-window-completed-meta">{formatBytes(size)}</span>
    </div>
    {showActions && <><button type="button" className="progress-window-icon progress-window-completed-open" disabled={disabled} onClick={() => onOpen("file")}
      title={t("progressWindow.openFile")} aria-label={t("progressWindow.openFileOf", { name: task.file_name })}><FileSymlink size={12} /></button>
    <button type="button" className="progress-window-icon progress-window-completed-folder" disabled={disabled} onClick={() => onOpen("folder")}
      title={t("progressWindow.openFolder")} aria-label={t("progressWindow.openFolderOf", { name: task.file_name })}><FolderOpen size={12} /></button></>}
  </li>;
}
