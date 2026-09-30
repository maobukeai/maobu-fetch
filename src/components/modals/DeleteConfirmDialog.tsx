/**
 * 删除确认对话框：所有会删除本地文件的任务删除操作（单个或批量）必须先经过本对话框，
 * 明确列出任务数量、文件数量（少量时列出文件名）并提示不可恢复（AGENTS.md §7）。
 *
 * 复用 common/Modal 的焦点圈定、Escape 关闭与确认按钮样式（confirm-dialog-*）。
 */
import { t } from "../../i18n";
import type { DownloadTask } from "../../types";
import {
  DELETE_CONFIRM_FILE_NAME_LIMIT,
  summarizeDeleteTargets,
  type DeleteSummary,
} from "../../delete-summary";
import { Modal } from "../common/Modal";

/** 一次删除确认请求：目标任务集合、是否删除本地文件，以及当前视图（影响文案）。 */
export interface DeleteConfirmRequest {
  taskIds: Set<string>;
  deleteFile: boolean;
  historyView: boolean;
}

/** 根据请求组合对话框标题：文件删除与历史彻底删除均为危险操作。 */
function resolveTitle(request: DeleteConfirmRequest): string {
  if (request.deleteFile) {
    return request.historyView
      ? t("deleteConfirm.historyFilesTitle")
      : t("deleteConfirm.filesTitle");
  }
  return request.historyView
    ? t("deleteConfirm.historyRecordsTitle")
    : t("deleteConfirm.recordsTitle");
}

/** 组装确认正文首段（不含未完成提示、文件清单与不可恢复声明）。 */
function resolveBody(request: DeleteConfirmRequest, summary: DeleteSummary): string {
  if (request.deleteFile) {
    return request.historyView
      ? t("deleteConfirm.historyFilesBody", {
          taskCount: summary.taskCount,
          fileCount: summary.fileCount,
        })
      : t("deleteConfirm.filesBody", {
          taskCount: summary.taskCount,
          fileCount: summary.fileCount,
        });
  }
  return request.historyView
    ? t("deleteConfirm.recordsBodyHistory", { taskCount: summary.taskCount })
    : t("deleteConfirm.recordsBody", { taskCount: summary.taskCount });
}

/** 仅当涉及文件总数 ≤ 上限时列出文件名，避免长清单刷屏。 */
function resolveFileNames(
  request: DeleteConfirmRequest,
  summary: DeleteSummary
): string[] {
  const listable =
    request.deleteFile &&
    summary.fileCount > 0 &&
    summary.fileCount <= DELETE_CONFIRM_FILE_NAME_LIMIT;
  return listable ? summary.fileNames : [];
}

export function DeleteConfirmDialog({
  request,
  tasks,
  onConfirm,
  onCancel,
}: {
  request: DeleteConfirmRequest;
  tasks: DownloadTask[];
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const summary = summarizeDeleteTargets(tasks, request.taskIds);
  const danger = request.deleteFile || request.historyView;

  const bodyLines: string[] = [resolveBody(request, summary)];
  if (summary.incompleteCount > 0) {
    bodyLines.push(
      t("deleteConfirm.incompleteNote", { count: summary.incompleteCount })
    );
  }
  const fileNames = resolveFileNames(request, summary);
  if (fileNames.length > 0) {
    bodyLines.push(t("deleteConfirm.fileListLabel"));
    for (const name of fileNames) bodyLines.push(`· ${name}`);
  }
  if (danger) {
    bodyLines.push(t("deleteConfirm.irreversible"));
  }

  return (
    <Modal
      title={resolveTitle(request)}
      onClose={onCancel}
      style={{ width: "440px" }}
    >
      <div className="confirm-dialog-body">
        <p>{bodyLines.join("\n")}</p>
        <div className="confirm-dialog-actions">
          <button
            className="confirm-btn-secondary"
            onClick={onCancel}
            autoFocus={danger}
          >
            {t("common.cancel")}
          </button>
          <button
            className={danger ? "confirm-btn-danger" : "confirm-btn-primary"}
            onClick={onConfirm}
            autoFocus={!danger}
          >
            {request.deleteFile
              ? t("deleteConfirm.confirmDeleteFiles")
              : t("deleteConfirm.confirmDelete")}
          </button>
        </div>
      </div>
    </Modal>
  );
}
