import React, { useMemo } from "react";
import { Folder, Layers, Pause, Play, X, Zap } from "lucide-react";
import { t } from "../../i18n";
import type { DownloadTask } from "../../types";
import { formatBytes, formatDuration } from "../../formatters";

export interface BatchSummary {
  batchId: string;
  title: string;
  tasks: DownloadTask[];
  totalTasks: number;
  completedTasks: number;
  failedTasks: number;
  activeTasks: number;
  pausedTasks: number;
  totalBytes: number;
  downloadedBytes: number;
  speed: number;
  etaSeconds?: number;
  isAllCompleted: boolean;
  isAllPaused: boolean;
  destinationDir?: string;
}

export function computeBatchSummaries(
  tasks: DownloadTask[],
  dismissedBatchIds: Set<string>
): BatchSummary[] {
  const groups = new Map<string, DownloadTask[]>();

  for (const task of tasks) {
    let key = task.batch_id?.trim();
    if (!key && task.source === "landisk_deconstructed") {
      // 兼容历史未记录显式 batch_id 的解构任务：按目标目录顶级文件夹归组
      const normalizedDest = task.destination.replace(/\\/g, "/");
      const parts = normalizedDest.split("/").filter(Boolean);
      key = parts.length > 0 ? parts[parts.length - 1] : "";
    }
    if (!key) continue;

    if (!groups.has(key)) {
      groups.set(key, []);
    }
    groups.get(key)!.push(task);
  }

  const summaries: BatchSummary[] = [];

  for (const [batchId, batchTasks] of groups.entries()) {
    // 至少 2 个任务才视为宏观批次展示聚合栏
    if (batchTasks.length < 2) continue;
    if (dismissedBatchIds.has(batchId)) continue;

    let completedTasks = 0;
    let failedTasks = 0;
    let activeTasks = 0;
    let pausedTasks = 0;
    let totalBytes = 0;
    let downloadedBytes = 0;
    let speed = 0;

    for (const t of batchTasks) {
      if (t.status === "completed") {
        completedTasks++;
      } else if (
        t.status === "failed" ||
        t.status === "cancelled" ||
        t.status === "interrupted"
      ) {
        failedTasks++;
      } else if (
        t.status === "downloading" ||
        t.status === "verifying"
      ) {
        activeTasks++;
      } else if (
        t.status === "paused" ||
        t.status === "paused-by-low-disk" ||
        t.status === "paused-by-metered" ||
        t.status === "waiting-network"
      ) {
        pausedTasks++;
      }

      totalBytes += t.total_bytes || 0;
      downloadedBytes += t.downloaded_bytes || 0;
      if (t.status === "downloading") {
        speed += t.speed || 0;
      }
    }

    const totalTasks = batchTasks.length;
    const isAllCompleted = completedTasks === totalTasks;
    const isAllPaused =
      pausedTasks + completedTasks === totalTasks && completedTasks < totalTasks;

    let etaSeconds: number | undefined;
    if (speed > 0 && totalBytes > downloadedBytes) {
      etaSeconds = Math.round((totalBytes - downloadedBytes) / speed);
    }

    const title = batchId;
    const destinationDir = batchTasks[0]?.destination;

    summaries.push({
      batchId,
      title,
      tasks: batchTasks,
      totalTasks,
      completedTasks,
      failedTasks,
      activeTasks,
      pausedTasks,
      totalBytes,
      downloadedBytes,
      speed,
      etaSeconds,
      isAllCompleted,
      isAllPaused,
      destinationDir,
    });
  }

  // 排序：未完成的排在前面，下载速度更快的优先
  return summaries.sort((a, b) => {
    if (a.isAllCompleted !== b.isAllCompleted) {
      return a.isAllCompleted ? 1 : -1;
    }
    return b.speed - a.speed;
  });
}

export interface BatchSummaryBarProps {
  summary: BatchSummary;
  onPauseAll: (summary: BatchSummary) => void;
  onResumeAll: (summary: BatchSummary) => void;
  onOpenFolder: (summary: BatchSummary) => void;
  onDismiss: (summary: BatchSummary) => void;
}

export const BatchSummaryBar: React.FC<BatchSummaryBarProps> = ({
  summary,
  onPauseAll,
  onResumeAll,
  onOpenFolder,
  onDismiss,
}) => {
  const percent = useMemo(() => {
    if (summary.totalBytes > 0) {
      return Math.min(
        100,
        Math.round((summary.downloadedBytes / summary.totalBytes) * 100)
      );
    }
    return Math.min(
      100,
      Math.round((summary.completedTasks / summary.totalTasks) * 100)
    );
  }, [summary]);

  const bytesText = useMemo(() => {
    if (summary.totalBytes > 0) {
      return t("batch.bytesProgress", {
        downloaded: formatBytes(summary.downloadedBytes),
        total: formatBytes(summary.totalBytes),
      });
    }
    return t("batch.bytesProgressNoTotal", {
      downloaded: formatBytes(summary.downloadedBytes),
    });
  }, [summary.downloadedBytes, summary.totalBytes]);

  return (
    <div
      className="batch-summary-bar"
      role="region"
      aria-label={`批量任务 ${summary.title}`}
    >
      <div className="batch-summary-left">
        <span className="batch-icon-badge" title={t("batch.title")}>
          <Layers className="batch-icon" size={14} />
        </span>
        <span className="batch-title" title={summary.title}>
          {summary.title}
        </span>
        <span className="batch-badge-count">
          {t("batch.itemsProgress", {
            done: summary.completedTasks,
            total: summary.totalTasks,
          })}
        </span>
      </div>

      <div className="batch-summary-middle">
        <div className="batch-progress-wrapper" title={`${percent}%`}>
          <div
            className={`batch-progress-fill ${
              summary.isAllCompleted ? "completed" : ""
            }`}
            style={{ width: `${percent}%` }}
          />
        </div>
        <span className="batch-bytes-text">{bytesText}</span>
        {summary.isAllCompleted ? (
          <span className="batch-status-badge completed">
            {t("batch.allCompleted")}
          </span>
        ) : summary.speed > 0 ? (
          <span className="batch-speed-text">
            <Zap size={12} className="batch-zap-icon" />
            {formatBytes(summary.speed)}/s
            {summary.etaSeconds !== undefined && (
              <span className="batch-eta-text">
                · {t("batch.eta", { eta: formatDuration(summary.etaSeconds) })}
              </span>
            )}
          </span>
        ) : summary.isAllPaused ? (
          <span className="batch-status-badge paused">
            {t("batch.allPaused")}
          </span>
        ) : null}
      </div>

      <div className="batch-summary-actions">
        {(summary.activeTasks > 0 ||
          summary.tasks.some((t) => t.status === "queued")) && (
          <button
            type="button"
            className="batch-btn batch-btn-action"
            onClick={() => onPauseAll(summary)}
            title={t("batch.pauseAll")}
            aria-label={t("batch.pauseAll")}
          >
            <Pause size={13} />
            <span>{t("batch.pauseAll")}</span>
          </button>
        )}
        {!summary.isAllCompleted &&
          (summary.pausedTasks > 0 || summary.failedTasks > 0) && (
            <button
              type="button"
              className="batch-btn batch-btn-action"
              onClick={() => onResumeAll(summary)}
              title={t("batch.resumeAll")}
              aria-label={t("batch.resumeAll")}
            >
              <Play size={13} />
              <span>{t("batch.resumeAll")}</span>
            </button>
          )}
        <button
          type="button"
          className="batch-btn batch-btn-icon"
          onClick={() => onOpenFolder(summary)}
          title={t("batch.openFolder")}
          aria-label={t("batch.openFolder")}
        >
          <Folder size={13} />
        </button>
        <button
          type="button"
          className="batch-btn batch-btn-icon batch-btn-close"
          onClick={() => onDismiss(summary)}
          title={t("batch.dismiss")}
          aria-label={t("batch.dismiss")}
        >
          <X size={13} />
        </button>
      </div>
    </div>
  );
};
