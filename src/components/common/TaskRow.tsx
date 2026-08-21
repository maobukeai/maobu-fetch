import { type CSSProperties, type MouseEvent } from "react";
import { Film, Image as ImageIcon, MoreHorizontal, Pause, Play, RefreshCw, Zap } from "lucide-react";
import { api } from "../../api";
import { t, useLocale } from "../../i18n";
import type { DownloadTask, Tag } from "../../types";
import { formatBytes, formatDate, formatDuration, getStatusText, hostOf } from "../../formatters";
import { FileIcon, inferCategory, TaskTagChips } from "./EmptyState";

export function isImageFile(fileName: string): boolean {
  const lower = fileName.toLowerCase();
  const imageExts = [
    ".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp", ".svg", ".ico", ".avif", ".tiff", ".tif", ".jfif"
  ];
  return imageExts.some((ext) => lower.endsWith(ext));
}

export function isVideoFile(fileName: string): boolean {
  const lower = fileName.toLowerCase();
  const mediaExts = [
    ".mp4", ".webm", ".mkv", ".mov", ".avi", ".flv", ".ts", ".wmv", ".m4v",
    ".mp3", ".flac", ".wav", ".aac", ".m4a", ".ogg",
  ];
  return mediaExts.some((ext) => lower.endsWith(ext));
}

export function isMediaTask(task: DownloadTask): boolean {
  if (task.media || isVideoFile(task.file_name)) return true;
  const mediaDomains = [
    "youtube.com",
    "youtu.be",
    "bilibili.com",
    "b23.tv",
    "douyin.com",
    "iesdouyin.com",
    "douyinvod.com",
    "vimeo.com",
    "tiktok.com",
    "twitter.com",
    "x.com",
    "weibo.com",
  ];
  try {
    const parsed = new URL(task.url);
    const hostname = parsed.hostname.toLowerCase();
    return mediaDomains.some(
      (domain) => hostname === domain || hostname.endsWith("." + domain)
    );
  } catch {
    return false;
  }
}

export const BT_ACTIVE_STATUSES = ["downloading", "connecting", "verifying", "extracting"];

export function btRuntimeActive(task: DownloadTask): boolean {
  return task.task_kind === "bt" && BT_ACTIVE_STATUSES.includes(task.status);
}

export function taskSpeedCellText(task: DownloadTask): string {
  if (task.task_kind !== "bt") {
    return task.status === "downloading" ? `${formatBytes(task.speed)}/s` : "—";
  }
  if (!btRuntimeActive(task)) return "—";
  const upload = task.bt_runtime?.upload_speed ?? 0;
  const down =
    task.status === "downloading" && task.speed > 0 ? `${formatBytes(task.speed)}/s` : "";
  const up = upload > 0 ? `↑${formatBytes(upload)}/s` : "";
  if (down && up) return `${down} ${up}`;
  return down || up || "—";
}

export function TaskRow({
  task,
  selected,
  showCompletedAt,
  taskTagList,
  notify,
  onSelect,
  onOpen,
  onContext,
  onMouseDown,
  onCheckboxMouseDown,
  onCheckboxMouseEnter,
}: {
  task: DownloadTask;
  selected: boolean;
  showCompletedAt: boolean;
  taskTagList: Tag[];
  notify: (text: string, kind?: "ok" | "error") => void;
  onSelect: () => void;
  onOpen: () => void;
  onContext: (event: MouseEvent) => void;
  onMouseDown: (task: DownloadTask, event: React.MouseEvent) => void;
  onCheckboxMouseDown: (event: React.MouseEvent) => void;
  onCheckboxMouseEnter: () => void;
}) {
  useLocale();
  const statusText = getStatusText();
  const progress = task.total_bytes
    ? Math.min(100, (task.downloaded_bytes / task.total_bytes) * 100)
    : 0;
  const speedMB = task.speed / (1024 * 1024);
  const stripeDuration =
    speedMB > 0 ? Math.max(0.25, Math.min(2.0, 1.5 / (speedMB + 0.5))) : 1.5;
  const isDownloading = [
    "downloading",
    "connecting",
    "verifying",
    "extracting",
  ].includes(task.status);
  const canControl = [
    "downloading",
    "connecting",
    "verifying",
    "extracting",
    "paused",
    "failed",
    "cancelled",
  ].includes(task.status);

  const handleAction = async (event: React.MouseEvent) => {
    event.stopPropagation();
    try {
      if (isDownloading) {
        await api.action(task.id, "pause");
      } else if (task.status === "failed") {
        await api.action(task.id, "retry");
      } else {
        await api.action(task.id, "resume");
      }
    } catch (e) {
      notify(String(e), "error");
    }
  };

  return (
    <div
      className={selected ? "task-row selected" : "task-row"}
      data-task-id={task.id}
      onDoubleClick={onOpen}
      onContextMenu={onContext}
      onMouseDown={(e) => onMouseDown(task, e)}
    >
      <label
        onMouseDown={(e) => {
          e.preventDefault();
          e.stopPropagation();
          onCheckboxMouseDown(e);
        }}
        onMouseEnter={onCheckboxMouseEnter}
        style={{ cursor: "pointer" }}
      >
        <input
          type="checkbox"
          aria-label={t("table.selectTaskAria", { name: task.file_name })}
          checked={selected}
          readOnly
        />
      </label>
      <div className="name-cell" onClick={onSelect}>
        <FileIcon category={task.category} fileName={task.file_name} />
        <div style={{ minWidth: 0, flex: 1 }}>
          <div className="name-title-row">
            <strong title={task.file_name}>
              {task.file_name ||
                (task.task_kind === "bt" ? t("table.btMetadataPending") : "—")}
            </strong>
            {task.task_kind === "bt" && (
              <span className="bt-task-badge" title={t("table.btBadgeTitle")}>
                <Zap size={9} strokeWidth={2.5} /> BT
              </span>
            )}
            {taskTagList.length > 0 && (
              <TaskTagChips tags={taskTagList} max={3} />
            )}
          </div>
          <small title={task.url}>
            {task.priority < 0
              ? t("details.highPriority")
              : task.priority > 0
              ? t("details.lowPriority")
              : ""}
            {task.task_kind === "bt" ? t("table.btSourceLabel") : hostOf(task.url)}
          </small>
        </div>
      </div>
      <span>
        {task.total_bytes > 0
          ? formatBytes(task.total_bytes)
          : task.downloaded_bytes > 0
          ? formatBytes(task.downloaded_bytes)
          : task.task_kind === "bt"
          ? t("table.btMetadataPending")
          : "—"}
      </span>
      <span className={`task-status ${task.status}`}>
        {task.status === "downloading" &&
        isMediaTask(task) &&
        task.downloaded_bytes === 0 &&
        task.active_connections === 0 &&
        !task.error
          ? statusText.parsing
          : task.status === "downloading" &&
            task.task_kind === "bt" &&
            (!task.bt_meta?.metadata_ready || task.total_bytes === 0)
          ? t("table.btMetadataFetching")
          : statusText[task.status]}
        {btRuntimeActive(task) && task.bt_runtime?.seeding && (
          <span className="bt-seed-badge" title={t("table.seedingTitle")}>
            {t("table.seeding")}
          </span>
        )}
        {canControl && (
          <button
            className="task-status-btn"
            onClick={handleAction}
            title={
              isDownloading
                ? t("details.taskStatusPause")
                : t("details.taskStatusResume")
            }
          >
            {isDownloading ? (
              <Pause size={10} strokeWidth={2.5} />
            ) : (
              <Play size={10} strokeWidth={2.5} />
            )}
          </button>
        )}
      </span>
      <span className="connection-count">
        {task.task_kind === "bt" ? (
          <span
            title={t("table.btConnTitle", {
              peers: task.bt_runtime?.num_peers ?? task.active_connections,
              seeds: task.bt_runtime?.num_seeds ?? 0,
            })}
          >
            {task.bt_runtime?.num_peers ?? task.active_connections}
            <small> {t("table.btPeerUnit")}</small>
          </span>
        ) : (
          <>
            {task.status === "downloading"
              ? `${task.active_connections}/${task.connection_count}`
              : task.connection_count}
            <small> {t("table.connectionUnit")}</small>
          </>
        )}
      </span>
      <div className="progress-cell">
        <div
          style={{
            position: "relative",
            overflow: "visible",
            flex: 1,
            display: "flex",
            alignItems: "center",
          }}
        >
          <div
            style={{
              flex: 1,
              height: "4px",
              overflow: "hidden",
              borderRadius: "2px",
              background: "var(--progress-track)",
              display: "flex",
            }}
          >
            <i
              style={
                {
                  width: `${progress}%`,
                  "--stripe-duration": `${stripeDuration}s`,
                } as CSSProperties
              }
              className={
                task.status === "downloading" && task.connection_count > 1
                  ? "multi-thread"
                  : ""
              }
            />
          </div>
          {task.status === "downloading" && task.connection_count > 1 && (
            <span
              className="speed-up-icon"
              style={{ left: `calc(${progress}% - 6px)` }}
              title={t("details.multiThread", {
                count: task.active_connections,
              })}
            >
              <Zap size={11} strokeWidth={2.5} />
            </span>
          )}
        </div>
        <span>
          {task.status === "completed"
            ? "100%"
            : task.total_bytes > 0
            ? `${progress.toFixed(0)}%`
            : task.task_kind === "bt"
            ? task.bt_meta?.metadata_ready
              ? "0%"
              : t("table.btMetadataPending")
            : `${progress.toFixed(0)}%`}
        </span>
      </div>
      <span
        title={
          btRuntimeActive(task) && task.bt_runtime?.seeding
            ? t("table.seedingTitle")
            : undefined
        }
      >
        {taskSpeedCellText(task)}
      </span>
      <span>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</span>
      <span>
        {formatDate(
          showCompletedAt ? task.completed_at ?? task.created_at : task.created_at
        )}
      </span>
      <div style={{ display: "inline-flex", alignItems: "center", gap: "2px" }}>
        {(task.task_kind === "bt" ||
          ["video", "audio"].includes(inferCategory(task.file_name, task.category))) &&
          (task.status === "completed" ||
            (task.status === "downloading" && task.downloaded_bytes > 0)) && (
            <button
              type="button"
              className="row-menu"
              onClick={(event) => {
                event.stopPropagation();
                if (task.status === "completed") {
                  const sep =
                    task.destination.endsWith("\\") || task.destination.endsWith("/")
                      ? ""
                      : "\\";
                  const fullPath = `${task.destination}${sep}${task.file_name}`;
                  void api
                    .openMediaPlayer(fullPath, task.file_name)
                    .catch((e) => notify(String(e), "error"));
                } else {
                  void api.openFile(task.id).catch((e) => notify(String(e), "error"));
                }
              }}
              title={
                task.status === "completed"
                  ? "使用猫步播放器播放"
                  : "🎬 边下边看 / 实时播放"
              }
              style={{ color: "var(--accent, #0078d4)" }}
            >
              <Film size={14} />
            </button>
          )}
        {isImageFile(task.file_name) && task.status === "completed" && (
          <button
            type="button"
            className="row-menu"
            onClick={(event) => {
              event.stopPropagation();
              const sep =
                task.destination.endsWith("\\") || task.destination.endsWith("/")
                  ? ""
                  : "\\";
              const fullPath = `${task.destination}${sep}${task.file_name}`;
              void api
                .openImageViewer(fullPath, task.file_name)
                .catch((e) => notify(String(e), "error"));
            }}
            title="使用猫步看图器查看"
            style={{ color: "var(--accent, #0078d4)" }}
          >
            <ImageIcon size={14} />
          </button>
        )}
        {(task.status === "cancelled" || task.status === "completed") && (
          <button
            type="button"
            className="row-menu"
            onClick={(event) => {
              event.stopPropagation();
              void api
                .action(task.id, "redownload")
                .then(() => {
                  notify("已重新加入下载队列并开始下载");
                })
                .catch((e) => notify(String(e), "error"));
            }}
            title={t("contextMenu.redownload") || "重新下载"}
            style={{ color: "var(--accent, #0078d4)" }}
          >
            <RefreshCw size={13} />
          </button>
        )}
        <button
          type="button"
          className="row-menu"
          onClick={(event) => {
            event.stopPropagation();
            onContext(event);
          }}
        >
          <MoreHorizontal size={15} />
        </button>
      </div>
    </div>
  );
}
