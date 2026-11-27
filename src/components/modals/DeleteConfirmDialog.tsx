/**
 * 删除确认对话框：所有会删除本地文件的任务删除操作（单个或批量）必须先经过本对话框，
 * 明确列出任务数量、文件数量（少量时列出文件名）并提示不可恢复（AGENTS.md §7）。
 *
 * 复用 common/Modal 的焦点圈定、Escape 关闭与确认按钮样式（confirm-dialog-*）。
 */
import { AlertTriangle } from "lucide-react";
import { t } from "../../i18n";
import type { DownloadTask } from "../../types";
import {
  DELETE_CONFIRM_FILE_NAME_LIMIT,
  summarizeDeleteTargets,
  type DeleteSummary,
} from "../../delete-summary";
import { Modal } from "../common/Modal";

export interface DeleteConfirmRequest {
  taskIds: Set<string>;
  deleteFile: boolean;
  historyView?: boolean;
}

function resolveTitle(request: DeleteConfirmRequest): string {
  if (request.historyView) {
    return request.deleteFile
      ? t("deleteConfirm.historyFilesTitle")
      : t("deleteConfirm.historyRecordsTitle");
  }
  return request.deleteFile
    ? t("deleteConfirm.filesTitle")
    : t("deleteConfirm.recordsTitle");
}

function resolveBody(
  request: DeleteConfirmRequest,
  summary: DeleteSummary
): string {
  if (request.historyView) {
    return request.deleteFile
      ? t("deleteConfirm.historyFilesBody", {
          taskCount: summary.taskCount,
          fileCount: summary.fileCount,
        })
      : t("deleteConfirm.recordsBodyHistory", {
          taskCount: summary.taskCount,
        });
  }
  return request.deleteFile
    ? t("deleteConfirm.filesBody", {
        taskCount: summary.taskCount,
        fileCount: summary.fileCount,
      })
    : t("deleteConfirm.recordsBody", { taskCount: summary.taskCount });
}

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
  const fileNames = resolveFileNames(request, summary);

  return (
    <Modal
      title={resolveTitle(request)}
      onClose={onCancel}
      dialogClassName="lm-dialog"
    >
      <div className="confirm-dialog-body lm-body">
        <p className="lm-lead">{resolveBody(request, summary)}</p>

        {summary.incompleteCount > 0 && (
          <p className="lm-sub-note">
            {t("deleteConfirm.incompleteNote", {
              count: summary.incompleteCount,
            })}
          </p>
        )}

        {fileNames.length > 0 && (
          <ul className="lm-file-list">
            {fileNames.map((name) => (
              <li key={name}>{name}</li>
            ))}
          </ul>
        )}

        {danger && (
          <div className="lm-warning-strip" role="note">
            <AlertTriangle size={14} />
            <span>{t("deleteConfirm.irreversible")}</span>
          </div>
        )}
      </div>

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
    </Modal>
  );
}
