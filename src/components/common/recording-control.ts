import type { DownloadTask } from "../../types";

export function canFinishRecording(task: DownloadTask): boolean {
  return task.task_kind !== "bt" && task.status === "downloading" && task.media?.is_live === true;
}
