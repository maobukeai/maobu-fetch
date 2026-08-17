import { useCallback, useEffect, useState } from "react";
import { Network, RefreshCw, CheckSquare, Square } from "lucide-react";
import { api } from "../api";
import type { BtFileEntry, DownloadTask } from "../types";
import { t } from "../i18n";

/**
 * BT/磁力任务详情面板（详情栏"连接"页签的 BT 分支，2026-08-16 批准）。
 *
 * 展示约束（AGENTS.md §3 BT/磁力内核）：
 * - 全部数据来自任务事件携带的 `bt_runtime`（aria2 真实状态），不模拟；
 * - 元数据获取阶段显示"待获取"，不显示伪造文件名/大小；
 * - 文件勾选调用 `bt_task_files` / `bt_select_files`，元数据未就绪时
 *   后端返回 `BT_METADATA_PENDING:` 前缀错误，此处转为可读提示。
 */
function formatBytes(value: number): string {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

export function BtDetailsPanel({ task, notify }: { task: DownloadTask; notify?: (text: string, kind?: "ok" | "error") => void }) {
  const runtime = task.bt_runtime ?? null;
  const metadataReady = task.bt_meta?.metadata_ready ?? false;
  // aria2 轮询随任务停止而停止：非活跃状态下统计是最后一次快照，需明示。
  const statsStale = !["downloading", "connecting", "verifying", "extracting"].includes(task.status);
  const [files, setFiles] = useState<BtFileEntry[] | null>(null);
  const [filesError, setFilesError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState<Set<number> | null>(null);
  const [saving, setSaving] = useState(false);

  const loadFiles = useCallback(async () => {
    setLoading(true);
    setFilesError(null);
    try {
      const entries = await api.btTaskFiles(task.id);
      setFiles(entries);
      setPending(new Set(entries.filter((entry) => entry.selected).map((entry) => entry.index)));
    } catch (reason) {
      const message = String(reason);
      setFiles(null);
      setFilesError(
        message.startsWith("BT_METADATA_PENDING")
          ? t("bt.metadataPending")
          : message
      );
    } finally {
      setLoading(false);
    }
  }, [task.id]);

  useEffect(() => {
    // 切换任务时重置文件列表；元数据就绪后自动加载一次。
    setFiles(null);
    setFilesError(null);
    setPending(null);
    if (metadataReady) void loadFiles();
  }, [task.id, metadataReady, loadFiles]);

  const toggleFile = (index: number) => {
    if (!pending) return;
    const next = new Set(pending);
    if (next.has(index)) {
      // 至少保留一个文件：取消最后一个勾选时明确反馈，不静默忽略。
      if (next.size <= 1) {
        notify?.(t("bt.keepOneFile"), "error");
        return;
      }
      next.delete(index);
    } else {
      next.add(index);
    }
    setPending(next);
  };

  const saveSelection = async () => {
    if (!pending) return;
    setSaving(true);
    try {
      await api.btSelectFiles(task.id, [...pending].sort((a, b) => a - b));
      await loadFiles();
    } catch (reason) {
      setFilesError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="bt-details-panel">
      <div className="connections-header">
        <span className="connections-title">{t("bt.connectionsTitle")}</span>
        <span className="connections-summary">
          {metadataReady
            ? `${runtime?.num_seeds ?? 0} ${t("bt.seedsUnit")} · ${runtime?.num_peers ?? 0} ${t("bt.peersUnit")}`
            : t("bt.metadataPending")}
        </span>
      </div>

      {runtime?.fetching_metadata ? (
        <div className="connections-empty">
          <Network size={28} strokeWidth={1.5} style={{ opacity: 0.4, marginBottom: 4 }} />
          <h3>{t("bt.metadataFetchingTitle")}</h3>
          <p>{t("bt.metadataFetchingBody")}</p>
        </div>
      ) : (
        <div className="bt-stats-row">
          <div className="bt-stat">
            <span className="bt-stat-label">{t("bt.seedsLabel")}</span>
            <span className="bt-stat-value">{runtime?.num_seeds ?? 0}</span>
          </div>
          <div className="bt-stat">
            <span className="bt-stat-label">{t("bt.peersLabel")}</span>
            <span className="bt-stat-value">{runtime?.num_peers ?? 0}</span>
          </div>
          <div className="bt-stat">
            <span className="bt-stat-label">{t("bt.uploadSpeedLabel")}</span>
            <span className="bt-stat-value">{runtime ? `${formatBytes(runtime.upload_speed)}/s` : "—"}</span>
          </div>
          <div className="bt-stat">
            <span className="bt-stat-label">{t("bt.downloadSpeedLabel")}</span>
            <span className="bt-stat-value">{task.status === "downloading" ? `${formatBytes(task.speed)}/s` : "—"}</span>
          </div>
          <div className="bt-stat">
            <span className="bt-stat-label">{t("bt.uploadedLabel")}</span>
            <span className="bt-stat-value">{runtime && runtime.uploaded_bytes ? formatBytes(runtime.uploaded_bytes) : "—"}</span>
          </div>
        </div>
      )}
      {statsStale && runtime && !runtime.fetching_metadata && (
        <p className="bt-files-hint" style={{ marginTop: -4 }}>{t("bt.statsStale")}</p>
      )}

      <div className="bt-files-section">
        <div className="bt-files-header">
          <span>{t("bt.filesTitle")}</span>
          <button
            type="button"
            className="bt-files-refresh"
            disabled={loading || saving || !metadataReady}
            title={metadataReady ? t("bt.filesRefreshTitle") : t("bt.metadataPending")}
            onClick={() => void loadFiles()}
          >
            <RefreshCw size={12} className={loading ? "spin" : undefined} /> {t("bt.filesRefresh")}
          </button>
          {files && pending && (
            <button
              type="button"
              className="bt-files-save"
              disabled={saving}
              onClick={() => void saveSelection()}
            >
              {t("bt.filesSave")}
            </button>
          )}
        </div>
        {!metadataReady && !filesError && (
          <p className="bt-files-hint">{t("bt.metadataPendingFilesHint")}</p>
        )}
        {filesError && <p className="bt-files-error">{filesError}</p>}
        {files && pending && (
          <div className="bt-files-list" role="listbox" aria-label={t("bt.filesTitle")}>
            {files.map((entry) => {
              const selected = pending.has(entry.index);
              return (
                <button
                  type="button"
                  key={entry.index}
                  className={`bt-file-item ${selected ? "selected" : ""}`}
                  onClick={() => toggleFile(entry.index)}
                  aria-pressed={selected}
                >
                  {selected ? <CheckSquare size={13} /> : <Square size={13} />}
                  <span className="bt-file-path" title={entry.path}>{entry.path}</span>
                  <span className="bt-file-size">{formatBytes(entry.length_bytes)}</span>
                </button>
              );
            })}
          </div>
        )}
      </div>

      <p className="bt-privacy-note">{t("bt.privacyNote")}</p>
    </div>
  );
}
