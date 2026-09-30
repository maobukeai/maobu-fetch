/**
 * 删除确认摘要（纯函数）：统计待删除任务集合的任务数、涉及本地文件数与文件名样本，
 * 供删除确认对话框明确告知"将影响什么"，避免误删本地文件（AGENTS.md §7）。
 */
import type { DownloadTask } from "./types";

/** 摘要所需的最小任务字段（结构化类型，便于测试与复用）。 */
export type DeleteSummaryTask = Pick<
  DownloadTask,
  "id" | "file_name" | "status" | "downloaded_bytes"
>;

/** 确认对话框中最多列出的文件名数量；文件总数超过该值时只显示数量。 */
export const DELETE_CONFIRM_FILE_NAME_LIMIT = 3;

/** 判定任务在磁盘上是否已有文件内容（已完成文件或已写入的分片数据）。 */
export function taskHasLocalFile(task: DeleteSummaryTask): boolean {
  return task.status === "completed" || task.downloaded_bytes > 0;
}

export interface DeleteSummary {
  /** 待删除任务数量。 */
  taskCount: number;
  /** 磁盘上已有文件内容的任务数量（删除文件时即涉及的文件数）。 */
  fileCount: number;
  /** 前 {@link DELETE_CONFIRM_FILE_NAME_LIMIT} 个涉及文件的任务文件名。 */
  fileNames: string[];
  /** 尚未完成的任务数量（其分片数据会被清理）。 */
  incompleteCount: number;
}

/** 统计待删除任务集合的影响范围。taskIds 为实际将被提交删除的任务 ID 集合。 */
export function summarizeDeleteTargets(
  tasks: ReadonlyArray<DeleteSummaryTask>,
  taskIds: ReadonlySet<string>
): DeleteSummary {
  let fileCount = 0;
  let incompleteCount = 0;
  const fileNames: string[] = [];
  for (const task of tasks) {
    if (!taskIds.has(task.id)) continue;
    if (task.status !== "completed") incompleteCount += 1;
    if (taskHasLocalFile(task)) {
      fileCount += 1;
      if (fileNames.length < DELETE_CONFIRM_FILE_NAME_LIMIT) {
        fileNames.push(task.file_name);
      }
    }
  }
  return { taskCount: taskIds.size, fileCount, fileNames, incompleteCount };
}
