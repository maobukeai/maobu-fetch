import { useCallback, useEffect, useState } from "react";
import { Network, RefreshCw, CheckSquare, Square, Play, Film, FileAudio, FileText, File } from "lucide-react";
import { api } from "../api";
import type { BtFileEntry, DownloadTask } from "../types";
import { t } from "../i18n";

function formatBytes(value: number): string {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

function getFileIcon(path: string) {
  if (/\.(mp4|mkv|avi|mov|flv|wmv|ts|webm)$/i.test(path)) return <Film size={14} style={{ color: "#0078d4", flexShrink: 0 }} />;
  if (/\.(mp3|flac|wav|m4a|aac|ogg)$/i.test(path)) return <FileAudio size={14} style={{ color: "#107c41", flexShrink: 0 }} />;
  if (/\.(srt|vtt|ass|txt|nfo|md)$/i.test(path)) return <FileText size={14} style={{ color: "#666", flexShrink: 0 }} />;
  return <File size={14} style={{ color: "#666", flexShrink: 0 }} />;
}

export function BtDetailsPanel({ task, notify }: { task: DownloadTask; notify?: (text: string, kind?: "ok" | "error") => void }) {
  const runtime = task.bt_runtime ?? null;
  const metadataReady = task.bt_meta?.metadata_ready ?? false;
  const isDownloading = ["downloading", "connecting", "verifying", "extracting"].includes(task.status);
  const statsStale = !isDownloading;
  const [files, setFiles] = useState<BtFileEntry[] | null>(null);
  const [filesError, setFilesError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState<Set<number> | null>(null);
  const [saving, setSaving] = useState(false);

  const loadFiles = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    setFilesError(null);
    try {
      const entries = await api.btTaskFiles(task.id);
      setFiles(entries);
      setPending((current) => {
        if (!current) {
          return new Set(entries.filter((entry) => entry.selected).map((entry) => entry.index));
        }
        return current;
      });
    } catch (reason) {
      const message = String(reason);
      if (!silent) {
        setFiles(null);
        setFilesError(
          message.startsWith("BT_METADATA_PENDING")
            ? t("bt.metadataPending")
            : message
        );
      }
    } finally {
      if (!silent) setLoading(false);
    }
  }, [task.id]);

  useEffect(() => {
    setFiles(null);
    setFilesError(null);
    setPending(null);
    if (metadataReady) void loadFiles();
  }, [task.id, metadataReady, loadFiles]);

  // 任务下载中时每 2 秒静默拉取一次每个文件的最新下载进度
  useEffect(() => {
    if (!metadataReady || !isDownloading) return;
    const timer = setInterval(() => {
      void loadFiles(true);
    }, 2000);
    return () => clearInterval(timer);
  }, [metadataReady, isDownloading, loadFiles]);

  const toggleFile = (index: number) => {
    if (!pending) return;
    const next = new Set(pending);
    if (next.has(index)) {
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
      notify?.("文件选择已更新", "ok");
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
        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          <span className="connections-summary">
            {`${runtime?.num_seeds ?? 0} ${t("bt.seedsUnit")} · ${runtime?.num_peers ?? 0} ${t("bt.peersUnit")}`}
          </span>
          <button
            type="button"
            className="bt-files-save"
            style={{ padding: "2px 8px", fontSize: "11px", display: "inline-flex", alignItems: "center", gap: "4px" }}
            title="调用系统默认播放器预览正在下载的媒体"
            onClick={() => void api.openFile(task.id).catch((e) => notify?.(String(e), "error"))}
          >
            <Play size={10} /> 边看边下
          </button>
        </div>
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
          <span>{t("bt.filesTitle")} ({files ? `${files.length} 个文件` : "加载中..."})</span>
          <div style={{ display: "inline-flex", gap: "8px", alignItems: "center" }}>
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
        </div>
        {!metadataReady && !filesError && (
          <p className="bt-files-hint">{t("bt.metadataPendingFilesHint")}</p>
        )}
        {filesError && <p className="bt-files-error">{filesError}</p>}
        {files && pending && (
          <div className="bt-files-list" role="listbox" aria-label={t("bt.filesTitle")} style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
            {files.map((entry) => {
              const selected = pending.has(entry.index);
              const isMedia = /\.(mp4|mkv|avi|mov|flv|wmv|ts|webm|mp3|flac|wav|m4a|aac)$/i.test(entry.path);
              const downloaded = entry.downloaded_bytes ?? (task.status === "completed" ? entry.length_bytes : 0);
              const pct = entry.length_bytes > 0 ? Math.min(100, Math.round((downloaded / entry.length_bytes) * 100)) : 0;

              return (
                <div
                  key={entry.index}
                  className={`bt-file-item ${selected ? "selected" : ""}`}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "4px",
                    padding: "6px 8px",
                    borderRadius: "5px",
                    border: "1px solid var(--border)",
                    background: selected ? "var(--bg-surface)" : "var(--bg-subtle, rgba(0,0,0,0.02))",
                    opacity: selected ? 1 : 0.6,
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                    <button
                      type="button"
                      style={{ background: "transparent", border: "none", cursor: "pointer", display: "inline-flex", alignItems: "center", color: "inherit", padding: 0 }}
                      onClick={() => toggleFile(entry.index)}
                      aria-pressed={selected}
                      title={selected ? "取消勾选此文件" : "勾选此文件下载"}
                    >
                      {selected ? <CheckSquare size={14} style={{ color: "var(--accent, #0078d4)" }} /> : <Square size={14} />}
                    </button>
                    {getFileIcon(entry.path)}
                    <span className="bt-file-path" title={entry.path} style={{ flex: 1, minWidth: 0, fontWeight: 500 }}>
                      {entry.path}
                    </span>
                    <span className="bt-file-size" style={{ fontSize: "11px", color: "var(--text-secondary, #666)", flexShrink: 0 }}>
                      {downloaded > 0 && downloaded < entry.length_bytes ? `${formatBytes(downloaded)} / ` : ""}
                      {formatBytes(entry.length_bytes)}
                    </span>
                    {isMedia && (
                      <button
                        type="button"
                        className="link-button"
                        style={{
                          fontSize: "11px",
                          color: "#fff",
                          background: "var(--accent, #0078d4)",
                          border: "none",
                          borderRadius: "4px",
                          padding: "2px 8px",
                          cursor: "pointer",
                          display: "inline-flex",
                          alignItems: "center",
                          gap: "3px",
                          flexShrink: 0,
                        }}
                        title="实时边下边播该视频"
                        onClick={(e) => {
                          e.stopPropagation();
                          void api.openFile(task.id, entry.path).catch((err) => notify?.(String(err), "error"));
                        }}
                      >
                        <Play size={10} fill="#fff" /> 播放
                      </button>
                    )}
                  </div>
                  {/* 各文件独立微型进度条 */}
                  {selected && (
                    <div style={{ display: "flex", alignItems: "center", gap: "8px", paddingLeft: "22px" }}>
                      <div style={{ flex: 1, height: "3px", background: "var(--progress-track, rgba(0,0,0,0.06))", borderRadius: "2px", overflow: "hidden" }}>
                        <div style={{ width: `${pct}%`, height: "100%", background: task.status === "completed" || pct === 100 ? "#107c41" : "var(--accent, #0078d4)", transition: "width 0.2s" }} />
                      </div>
                      <span style={{ fontSize: "10px", color: "var(--text-secondary, #777)", minWidth: "28px", textAlign: "right" }}>
                        {pct}%
                      </span>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      <p className="bt-privacy-note">{t("bt.privacyNote")}</p>
    </div>
  );
}
