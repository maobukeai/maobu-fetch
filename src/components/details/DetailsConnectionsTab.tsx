import { useEffect, useState } from "react";
import { Network } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { isDesktop } from "../../api";
import { t, useLocale } from "../../i18n";
import type { ConnectionState, DownloadTask, SegmentStatus, TaskConnectionsEvent } from "../../types";
import { formatBytes, getConnectionStateLabel } from "../../formatters";

function useConnectionStateLabel(): Record<ConnectionState, string> {
  useLocale();
  return getConnectionStateLabel();
}

export function DetailsConnectionsTab({ task }: { task: DownloadTask }) {
  const [segments, setSegments] = useState<Record<string, SegmentStatus>>({});
  const [lastTimestamp, setLastTimestamp] = useState<number | undefined>();
  const taskId = task.id;
  const connectionStateLabel = useConnectionStateLabel();

  useEffect(() => {
    setSegments({});
    setLastTimestamp(undefined);
    if (!isDesktop()) return;

    let cancelled = false;
    const unlisten: Array<() => void> = [];

    listen<TaskConnectionsEvent>("task-connections", (event) => {
      if (cancelled) return;
      if (event.payload.task_id !== taskId) return;
      const next: Record<string, SegmentStatus> = {};
      for (const seg of event.payload.segments) {
        next[seg.segment_id] = seg;
      }
      setSegments(next);
      setLastTimestamp(event.payload.timestamp);
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten.push(fn);
      }
    });

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [taskId]);

  const list = Object.values(segments).sort(
    (a, b) => Number(a.segment_id) - Number(b.segment_id)
  );
  const displayList: SegmentStatus[] =
    list.length > 0
      ? list
      : task.segments.map((seg) => ({
          segment_id: String(seg.index),
          start_offset: seg.start_byte,
          downloaded_bytes: seg.downloaded_bytes,
          total_bytes: seg.end_byte - seg.start_byte + 1,
          speed: 0,
          state:
            seg.status === "completed"
              ? "completed"
              : seg.status === "failed"
              ? "failed"
              : "paused",
          retry_count: 0,
          error: null,
        }));

  const totalCount = displayList.length;
  const completedCount = displayList.filter(
    (s) => s.state === "completed"
  ).length;
  const activeCount = displayList.filter(
    (s) =>
      s.state === "downloading" ||
      s.state === "connecting" ||
      s.state === "retrying"
  ).length;
  const failedCount = displayList.filter((s) => s.state === "failed").length;
  const totalDownloaded = displayList.reduce(
    (sum, s) => sum + s.downloaded_bytes,
    0
  );
  const totalBytes = displayList.reduce((sum, s) => sum + s.total_bytes, 0);
  const overallPercent =
    totalBytes > 0
      ? Math.min(100, Math.round((totalDownloaded / totalBytes) * 100))
      : 0;
  const live = task.status === "downloading";

  if (totalCount === 0) {
    return (
      <div className="connections-empty">
        <Network
          size={28}
          strokeWidth={1.5}
          style={{ opacity: 0.4, marginBottom: 4 }}
        />
        <h3>暂无分片连接</h3>
        <p>该任务未启用多连接，或尚未开始下载。</p>
      </div>
    );
  }

  return (
    <div className="connections-panel">
      <div className="connections-header">
        <span className="connections-title">分片连接</span>
        <span className="connections-summary">
          {completedCount}/{totalCount} 已完成 · {activeCount} 活跃
          {failedCount > 0 ? ` · ${failedCount} 失败` : ""}
        </span>
      </div>
      <div className="connections-overall">
        <div className="connections-overall-bar">
          <i style={{ width: `${overallPercent}%` }} />
        </div>
        <span className="connections-overall-text">
          {formatBytes(totalDownloaded)} / {formatBytes(totalBytes)} (
          {overallPercent}%)
        </span>
      </div>
      <div className="connections-list">
        {displayList.map((seg) => {
          const percent =
            seg.total_bytes > 0
              ? Math.min(
                  100,
                  Math.round((seg.downloaded_bytes / seg.total_bytes) * 100)
                )
              : 0;
          const meta: string[] = [];
          if (seg.speed > 0) meta.push(`${formatBytes(seg.speed)}/s`);
          if (seg.retry_count > 0)
            meta.push(t("connections.retryCount", { count: seg.retry_count }));
          return (
            <div
              key={seg.segment_id}
              className={`connection-item state-${seg.state}`}
            >
              <div className="connection-row">
                <span className="connection-index">#{seg.segment_id}</span>
                <span className="connection-state-badge">
                  {connectionStateLabel[seg.state]}
                </span>
                <span className="connection-bytes">
                  {formatBytes(seg.downloaded_bytes)} /{" "}
                  {formatBytes(seg.total_bytes)}
                </span>
                <span className="connection-percent">{percent}%</span>
              </div>
              <div className="connection-bar">
                <i style={{ width: `${percent}%` }} />
              </div>
              {(meta.length > 0 || (seg.state === "failed" && seg.error)) && (
                <div className="connection-meta">
                  {meta.map((m, i) => (
                    <span key={i}>{m}</span>
                  ))}
                  {seg.state === "failed" && seg.error && (
                    <span className="connection-error">{seg.error}</span>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
      <div className="connections-footer">
        {live
          ? lastTimestamp
            ? t("connections.realtime", {
                time: new Date(lastTimestamp).toLocaleTimeString(),
              })
            : t("connections.waitingFirstPush")
          : t("connections.stopped")}
      </div>
    </div>
  );
}
