import { useMemo, type ReactNode } from "react";
import {
  AlertCircle,
  Archive,
  ChevronDown,
  ChevronUp,
  ChevronsDown,
  ChevronsUp,
  CirclePause,
  Copy,
  ExternalLink,
  Film,
  FolderOpen,
  Gauge,
  Image as ImageIcon,
  Info,
  Pause,
  Play,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { api } from "../../api";
import { t, useLocale } from "../../i18n";
import type { CompletionAction, DownloadTask } from "../../types";
import {
  clampPriority,
  formatBytes,
  MIN_PRIORITY,
  MAX_PRIORITY,
  PRIORITY_STEP,
} from "../../formatters";
import { inferCategory } from "./EmptyState";
import { isImageFile, isMediaTask, isVideoFile } from "./TaskRow";
import { isLanDiskUrl } from "../../services/landisk";

export function ContextMenu({
  x,
  y,
  task,
  selectedTaskIds,
  allTasks = [],
  downloadDir,
  close,
  notify,
  onSetSpeedLimit,
  onDelete,
  onViewDetails,
  onRefreshUrl,
}: {
  x: number;
  y: number;
  task: DownloadTask;
  selectedTaskIds?: Set<string>;
  allTasks?: DownloadTask[];
  downloadDir?: string;
  close: () => void;
  notify: (text: string, kind?: "ok" | "error") => void;
  onSetSpeedLimit: (task: DownloadTask) => void;
  onDelete: (taskIds: Set<string>, deleteFile: boolean) => void;
  onViewDetails?: () => void;
  onRefreshUrl?: (task: DownloadTask) => void;
}) {
  useLocale();
  const targetTaskIds = useMemo(() => {
    if (
      selectedTaskIds &&
      selectedTaskIds.has(task.id) &&
      selectedTaskIds.size > 1
    ) {
      return selectedTaskIds;
    }
    return new Set([task.id]);
  }, [selectedTaskIds, task.id]);

  const targetTasks = useMemo(() => {
    return allTasks.filter((t) => targetTaskIds.has(t.id));
  }, [allTasks, targetTaskIds]);

  const isLanDiskTask =
    task.source === "landisk_deconstructed" ||
    task.source === "landisk_zip" ||
    isLanDiskUrl(task.url);

  const lanDiskDirectoryToPack = useMemo(() => {
    if (!isLanDiskTask) return null;
    const normDest = task.destination.replace(/\\/g, "/").replace(/\/+$/, "");
    if (!normDest || normDest === "/" || /^[a-zA-Z]:$/.test(normDest)) {
      return null;
    }
    const normBase = downloadDir
      ? downloadDir.replace(/\\/g, "/").replace(/\/+$/, "")
      : "";

    // 若任务目标目录等于全局下载根目录，说明是平铺在根目录的单文件，无独立任务子目录
    if (normBase && normDest.toLowerCase() === normBase.toLowerCase()) {
      return null;
    }

    // 若位于全局下载根目录下，提取其顶层子目录（即该批次/目录任务的根目录）
    if (normBase && normDest.toLowerCase().startsWith(normBase.toLowerCase() + "/")) {
      const rel = normDest.slice(normBase.length + 1);
      const topFolder = rel.split("/")[0];
      if (!topFolder) return null;
      return {
        dirPath: `${normBase}/${topFolder}`,
        folderName: topFolder,
        parentDir: normBase,
      };
    }

    // 自定义下载目录（非全局下载目录且非盘符根目录）
    const lastSlash = normDest.lastIndexOf("/");
    if (lastSlash <= 0) return null;
    const parent = normDest.slice(0, lastSlash);
    const folder = normDest.slice(lastSlash + 1);
    if (!folder) return null;
    return {
      dirPath: normDest,
      folderName: folder,
      parentDir: parent,
    };
  }, [isLanDiskTask, task.destination, downloadDir]);

  const countTag =
    targetTaskIds.size > 1
      ? t("contextMenu.countSuffix", { count: targetTaskIds.size })
      : "";

  const action = async (value: string) => {
    try {
      for (const id of targetTaskIds) {
        await api.action(id, value);
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      close();
    }
  };

  const update = async (options: {
    priority?: number;
    perTaskSpeedLimit?: number;
    completionAction?: CompletionAction;
  }) => {
    try {
      for (const id of targetTaskIds) {
        await api.updateTaskOptions(id, options);
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      close();
    }
  };

  const changeSpeedLimit = () => {
    onSetSpeedLimit(task);
    close();
  };

  const copyText = async (label: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      notify(t("toasts.copiedLabel", { label }));
    } catch (error) {
      notify(
        `${t("toasts.copyLabelFailed", { label })}：${String(error)}`,
        "error"
      );
    } finally {
      close();
    }
  };

  const copyUrls = async () => {
    try {
      const list = targetTasks.length > 0 ? targetTasks : [task];
      const text = list.map((t) => t.url).join("\n");
      await navigator.clipboard.writeText(text);
      notify(
        list.length > 1
          ? t("toasts.linksCopied", { count: list.length })
          : t("toasts.linkCopied")
      );
    } catch (error) {
      notify(`${t("toasts.copyLinkFailed")}：${String(error)}`, "error");
    } finally {
      close();
    }
  };

  const buildFilePath = () => {
    const sep =
      task.destination.endsWith("\\") || task.destination.endsWith("/")
        ? ""
        : "\\";
    return `${task.destination}${sep}${task.file_name}`;
  };

  const showDiagnosis = () => {
    const detail = task.error || t("toasts.diagnosisNoDetail");
    notify(t("toasts.diagnosisPrefix", { detail }), "error");
    close();
  };

  const confirmDelete = (deleteFile: boolean) => {
    onDelete(targetTaskIds, deleteFile);
    close();
  };

  const menuWidth = 220;
  const itemHeight = 30;
  const separatorHeight = 9;
  const padding = 8;

  const sections: ReactNode[] = [];
  const pushSep = () =>
    sections.push(
      <div key={`sep-${sections.length}`} className="context-menu-separator" />
    );

  switch (task.status) {
    case "downloading":
    case "verifying":
    case "waiting-network":
      sections.push(
        <button key="pause" onClick={() => void action("pause")}>
          <Pause size={13} />
          {t("contextMenu.pause")}
        </button>
      );
      const isAudioVisual =
        task.task_kind === "bt" ||
        ["video", "audio"].includes(inferCategory(task.file_name, task.category));
      if (isAudioVisual) {
        sections.push(
          <button
            key="stream-play"
            onClick={() =>
              void api
                .openFile(task.id)
                .then(close)
                .catch((e) => notify(String(e), "error"))
            }
          >
            <Film size={13} />
            {t("contextMenu.streamPlay")}
          </button>
        );
      }
      break;
    case "paused":
      sections.push(
        <button key="resume" onClick={() => void action("resume")}>
          <Play size={13} />
          {t("contextMenu.resume")}
        </button>
      );
      break;
    case "interrupted":
      sections.push(
        <button key="resume" onClick={() => void action("resume")}>
          <Play size={13} />
          {t("contextMenu.resume")}
        </button>
      );
      break;
    case "paused-by-low-disk":
      sections.push(
        <button key="resume" onClick={() => void action("resume")}>
          <Play size={13} />
          {t("contextMenu.resume")}
        </button>
      );
      sections.push(
        <button key="change-dir" disabled title={t("contextMenu.changeDirTitle")}>
          <FolderOpen size={13} />
          {t("contextMenu.changeDir")}
        </button>
      );
      break;
    case "paused-by-metered":
      sections.push(
        <button key="resume" onClick={() => void action("resume")}>
          <Play size={13} />
          {t("contextMenu.resumeDownload")}
        </button>
      );
      break;
    case "failed":
      sections.push(
        <button key="diagnose" onClick={() => showDiagnosis()}>
          <AlertCircle size={13} />
          {t("contextMenu.diagnose")}
        </button>
      );
      sections.push(
        <button key="retry" onClick={() => void action("retry")}>
          <RefreshCw size={13} />
          {t("contextMenu.retry")}
        </button>
      );
      break;
    case "remote-changed":
      sections.push(
        <button key="redownload" onClick={() => void action("redownload")}>
          <RefreshCw size={13} />
          {t("contextMenu.redownload")}
        </button>
      );
      sections.push(
        <button key="keep-cancel" onClick={() => void action("cancel")}>
          <CirclePause size={13} />
          {t("contextMenu.keepOldFile")}
        </button>
      );
      break;
    case "completed":
      if (targetTaskIds.size <= 1) {
        if (isVideoFile(task.file_name) || isMediaTask(task)) {
          sections.push(
            <button
              key="open-media-player"
              onClick={() => {
                const fullPath = buildFilePath();
                void api
                  .openMediaPlayer(fullPath, task.file_name)
                  .then(close)
                  .catch((e) => notify(String(e), "error"));
              }}
            >
              <Film size={13} className="text-sky-400" />
              {t("contextMenu.playWithBuiltin") || "内置播放器播放"}
            </button>
          );
        }
        if (isImageFile(task.file_name)) {
          sections.push(
            <button
              key="open-image-viewer"
              onClick={() => {
                const fullPath = buildFilePath();
                void api
                  .openImageViewer(fullPath, task.file_name)
                  .then(close)
                  .catch((e) => notify(String(e), "error"));
              }}
            >
              <ImageIcon size={13} className="text-sky-400" />
              {"内置看图器查看"}
            </button>
          );
        }
        sections.push(
          <button
            key="open-file"
            onClick={() =>
              void api
                .openFile(task.id)
                .then(close)
                .catch((e) => notify(String(e), "error"))
            }
          >
            <ExternalLink size={13} />
            {t("contextMenu.openFile")}
          </button>
        );
        sections.push(
          <button
            key="open-folder"
            onClick={() =>
              void api
                .openFolder(task.id)
                .then(close)
                .catch((e) => notify(String(e), "error"))
            }
          >
            <FolderOpen size={13} />
            {t("contextMenu.openFolder")}
          </button>
        );
      }
      sections.push(
        <button
          key="copy-path"
          onClick={() => {
            if (targetTaskIds.size <= 1) {
              void copyText(t("contextMenu.filePathLabel"), buildFilePath());
            } else {
              const paths = targetTasks
                .map((t) => {
                  const sep =
                    t.destination.endsWith("\\") || t.destination.endsWith("/")
                      ? ""
                      : "\\";
                  return `${t.destination}${sep}${t.file_name}`;
                })
                .join("\n");
              void copyText(t("contextMenu.filePathLabel"), paths);
            }
          }}
        >
          <Copy size={13} />
          {t("contextMenu.copyFilePath")}
          {countTag}
        </button>
      );
      if (targetTaskIds.size <= 1) {
        sections.push(
          <button
            key="verify"
            onClick={() =>
              void api
                .verify(task.id)
                .then(() => {
                  notify(t("toasts.verifyDone"));
                  close();
                })
                .catch((e) => notify(String(e), "error"))
            }
          >
            <ShieldCheck size={13} />
            {t("contextMenu.verifySha256")}
          </button>
        );
      }
      if (lanDiskDirectoryToPack) {
        sections.push(
          <button
            key="pack-landisk-zip"
            onClick={async () => {
              try {
                // 检查该目录下是否有同批次任务仍在下载或处理中
                const normDirPath = lanDiskDirectoryToPack.dirPath.toLowerCase();
                const hasActiveSiblings = allTasks.some((t) => {
                  if (t.id === task.id) return false;
                  const tNorm = t.destination.replace(/\\/g, "/").toLowerCase();
                  const isInDir =
                    tNorm === normDirPath || tNorm.startsWith(normDirPath + "/");
                  const isActive =
                    t.status === "downloading" ||
                    t.status === "queued" ||
                    t.status === "waiting-network" ||
                    t.status === "verifying";
                  return isInDir && isActive;
                });

                if (hasActiveSiblings) {
                  notify(
                    "该目录下仍有文件正在下载中，请等待全部完成后再打包",
                    "error"
                  );
                  return;
                }

                const sep = task.destination.includes("\\") ? "\\" : "/";
                const nativeDirPath = lanDiskDirectoryToPack.dirPath.replace(/\//g, sep);
                const nativeParentDir = lanDiskDirectoryToPack.parentDir.replace(/\//g, sep);
                const zipPath = `${nativeParentDir}${sep}${lanDiskDirectoryToPack.folderName}.zip`;

                notify(`正在将目录 ${lanDiskDirectoryToPack.folderName} 打包为 ZIP...`);
                const size = await api.landiskPackZip(nativeDirPath, zipPath);
                notify(
                  `已成功打包为 ZIP：${lanDiskDirectoryToPack.folderName}.zip (${formatBytes(size)})`,
                  "ok"
                );
              } catch (error) {
                notify(`打包 ZIP 失败：${String(error)}`, "error");
              } finally {
                close();
              }
            }}
          >
            <Archive size={13} style={{ color: "#10b981" }} />
            {"打包任务目录为 ZIP"}
          </button>
        );
      }
      sections.push(
        <button key="redownload" onClick={() => void action("redownload")}>
          <RefreshCw size={13} />
          {t("contextMenu.redownload")}
          {countTag}
        </button>
      );
      break;
    case "cancelled":
      sections.push(
        <button key="redownload" onClick={() => void action("redownload")}>
          <RefreshCw size={13} />
          {t("contextMenu.redownload")}
          {countTag}
        </button>
      );
      if (targetTaskIds.size <= 1) {
        sections.push(
          <button
            key="open-folder"
            onClick={() =>
              void api
                .openFolder(task.id)
                .then(close)
                .catch((e) => notify(String(e), "error"))
            }
          >
            <FolderOpen size={13} />
            {t("contextMenu.openFolder")}
          </button>
        );
      }
      sections.push(
        <button
          key="copy-path"
          onClick={() => {
            if (targetTaskIds.size <= 1) {
              void copyText(t("contextMenu.filePathLabel"), buildFilePath());
            } else {
              const paths = targetTasks
                .map((t) => {
                  const sep =
                    t.destination.endsWith("\\") || t.destination.endsWith("/")
                      ? ""
                      : "\\";
                  return `${t.destination}${sep}${t.file_name}`;
                })
                .join("\n");
              void copyText(t("contextMenu.filePathLabel"), paths);
            }
          }}
        >
          <Copy size={13} />
          {t("contextMenu.copyFilePath")}
          {countTag}
        </button>
      );
      break;
    case "queued":
    case "scheduled":
    default:
      break;
  }

  if (!["cancelled", "completed"].includes(task.status)) {
    sections.push(
      <div key="priority-row" className="context-menu-row-item">
        <span className="context-menu-row-label">
          {t("contextMenu.queueOrder")}
        </span>
        <div className="context-menu-row-buttons">
          <button
            onClick={() => void update({ priority: MIN_PRIORITY })}
            title={t("contextMenu.toTop")}
          >
            <ChevronsUp size={13} />
          </button>
          <button
            onClick={() =>
              void update({
                priority: clampPriority(task.priority - PRIORITY_STEP),
              })
            }
            title={t("contextMenu.moveUp")}
          >
            <ChevronUp size={13} />
          </button>
          <button
            onClick={() =>
              void update({
                priority: clampPriority(task.priority + PRIORITY_STEP),
              })
            }
            title={t("contextMenu.moveDown")}
          >
            <ChevronDown size={13} />
          </button>
          <button
            onClick={() => void update({ priority: MAX_PRIORITY })}
            title={t("contextMenu.toBottom")}
          >
            <ChevronsDown size={13} />
          </button>
        </div>
      </div>
    );
    sections.push(
      <button key="speed-limit" onClick={() => void changeSpeedLimit()}>
        <Gauge size={13} />
        {t("contextMenu.speedLimit", {
          value: task.per_task_speed_limit
            ? `${Math.round(task.per_task_speed_limit / 1024)} KB/s`
            : t("details.noSpeedLimit"),
        })}
      </button>
    );
    sections.push(
      <button
        key="completion"
        onClick={() =>
          void update({
            completionAction:
              task.completion_action === "open-folder"
                ? "none"
                : "open-folder",
          })
        }
      >
        <FolderOpen size={13} />
        {task.completion_action === "open-folder"
          ? t("contextMenu.completionOpenFolderOff")
          : t("contextMenu.completionOpenFolderOn")}
      </button>
    );
  }

  if (onRefreshUrl && targetTaskIds.size <= 1 && task.status !== "completed") {
    sections.push(
      <button
        key="refresh-url"
        onClick={() => {
          onRefreshUrl(task);
          close();
        }}
      >
        <RefreshCw size={13} />
        {t("contextMenu.refreshUrl") || "刷新下载链接"}
      </button>
    );
  }

  if (onViewDetails) {
    sections.push(
      <button
        key="view-details"
        onClick={() => {
          onViewDetails();
          close();
        }}
      >
        <Info size={13} />
        {t("toasts.viewDetails")}
      </button>
    );
  }

  if (sections.length > 0) pushSep();
  sections.push(
    <button key="copy-url" onClick={() => void copyUrls()}>
      <Copy size={13} />
      {t("contextMenu.copyLink")}
      {countTag}
    </button>
  );

  const isHistoryOrCancelled = task.status === "cancelled";
  pushSep();
  sections.push(
    <button
      key="delete-record"
      className="danger"
      onClick={() => void confirmDelete(false)}
    >
      <Trash2 size={13} />
      {isHistoryOrCancelled
        ? t("dialogs.deletePermanently") || "彻底删除记录"
        : t("dialogs.deleteRecordOnly")}
      {countTag}
    </button>
  );
  sections.push(
    <button
      key="delete-file"
      className="danger"
      onClick={() => void confirmDelete(true)}
    >
      <Trash2 size={13} />
      {isHistoryOrCancelled
        ? t("dialogs.deletePermanentlyAndFile") || "彻底删除记录与本地文件"
        : t("dialogs.deleteRecordAndFile")}
      {countTag}
    </button>
  );

  let separatorCount = 0;
  for (const node of sections) {
    if ((node as any)?.props?.className === "context-menu-separator")
      separatorCount++;
  }
  const buttonCount = sections.length - separatorCount;
  const menuHeight =
    buttonCount * itemHeight + separatorCount * separatorHeight + padding;
  const safeX = Math.max(8, Math.min(x, window.innerWidth - menuWidth - 8));
  const safeY = Math.max(8, Math.min(y, window.innerHeight - menuHeight - 8));

  return (
    <div
      className="context-menu"
      style={{ left: safeX, top: safeY, minWidth: menuWidth }}
      onClick={(e) => e.stopPropagation()}
    >
      {sections}
    </div>
  );
}
