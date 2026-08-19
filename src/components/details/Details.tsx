import { useEffect, useRef, useState } from "react";
import { Info, X } from "lucide-react";
import { api } from "../../api";
import { t, useLocale } from "../../i18n";
import type { DownloadTask, PrecheckResult, TaskStatus } from "../../types";
import { BtDetailsPanel } from "../BtDetailsPanel";
import { DiagnosisPanel } from "../DiagnosisPanel";
import { PrecheckPanel } from "../PrecheckPanel";
import { DetailsConnectionsTab } from "./DetailsConnectionsTab";
import { DetailsInfoTab } from "./DetailsInfoTab";
import { SpeedHistoryCard } from "./SpeedHistoryCard";

const DIAGNOSIS_TAB_STATUSES: TaskStatus[] = [
  "failed",
  "interrupted",
  "remote-changed",
  "paused-by-low-disk",
];

const SPEED_HISTORY_MAX = 300;

export function Details({
  task,
  onClose,
  notify,
  selectedCount,
  onOpenProxySettings,
  onOpenYouTubeModal,
  onOpenRefreshUrl,
  onTagsChanged,
}: {
  task?: DownloadTask;
  onClose: () => void;
  notify: (text: string, kind?: "ok" | "error") => void;
  selectedCount: number;
  onOpenProxySettings?: () => void;
  onOpenYouTubeModal?: () => void;
  onOpenRefreshUrl?: (task: DownloadTask) => void;
  onTagsChanged?: () => void;
}) {
  useLocale();
  const [showMore, setShowMore] = useState(false);
  const [tab, setTab] = useState<"info" | "diagnosis" | "precheck" | "connections">("info");
  const [precheck, setPrecheck] = useState<{
    loading: boolean;
    result?: PrecheckResult;
    error?: string;
  }>({ loading: false });

  const taskId = task?.id;
  const taskStatus = task?.status;

  const activeRequestRef = useRef<string | null>(null);

  useEffect(() => {
    activeRequestRef.current = null;
    setPrecheck({ loading: false });
    if (taskStatus && DIAGNOSIS_TAB_STATUSES.includes(taskStatus)) {
      setTab("diagnosis");
    } else {
      setTab("info");
    }
  }, [taskId]);

  useEffect(() => {
    if (taskId === undefined) return;
    activeRequestRef.current = null;
    setPrecheck({ loading: false });
    if (taskStatus && DIAGNOSIS_TAB_STATUSES.includes(taskStatus)) {
      setTab("diagnosis");
    }
  }, [taskStatus]);

  const runPrecheck = async () => {
    if (!task) return;
    const reqId = Math.random().toString(36).slice(2);
    activeRequestRef.current = reqId;
    setPrecheck({ loading: true });

    try {
      const timeoutPromise = new Promise<never>((_, reject) =>
        setTimeout(
          () => reject(new Error("前端预检等待超时（20 秒），请重试或检查网络")),
          20_000
        )
      );
      const result = await Promise.race([
        api.precheck({
          url: task.url,
          target_directory: task.destination,
          suggested_filename: task.file_name,
          headers:
            Object.keys(task.headers || {}).length > 0 ? task.headers : undefined,
          proxy_override: task.proxy_override,
          proxy_auth: task.proxy_auth,
        }),
        timeoutPromise,
      ]);
      if (activeRequestRef.current === reqId) {
        setPrecheck({ loading: false, result });
      }
    } catch (err) {
      if (activeRequestRef.current === reqId) {
        setPrecheck({ loading: false, error: String(err) });
      }
    }
  };

  useEffect(() => {
    if (
      tab === "precheck" &&
      task &&
      !precheck.result &&
      !precheck.loading &&
      !precheck.error
    ) {
      void runPrecheck();
    }
  }, [tab, taskId, precheck.loading, precheck.result, precheck.error]);

  const [speedHistory, setSpeedHistory] = useState<number[]>([]);
  const latestSpeedRef = useRef(0);
  const activeTaskIdRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    latestSpeedRef.current = task?.speed ?? 0;
  }, [task?.speed]);
  useEffect(() => {
    activeTaskIdRef.current = taskId;
    setSpeedHistory([]);
  }, [taskId]);
  useEffect(() => {
    const timer = window.setInterval(() => {
      if (activeTaskIdRef.current === undefined) return;
      setSpeedHistory((prev) => {
        const next =
          prev.length >= SPEED_HISTORY_MAX
            ? prev.slice(prev.length - SPEED_HISTORY_MAX + 1)
            : prev.slice();
        next.push(latestSpeedRef.current);
        return next;
      });
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  if (!task) {
    return (
      <aside className="details-pane">
        <div className="details-header">
          <h2>{t("details.title")}</h2>
          <button onClick={onClose} title={t("common.close")}>
            <X size={14} />
          </button>
        </div>
        <div
          className="details-scroll"
          style={{
            justifyContent: "center",
            alignItems: "center",
            color: "var(--muted)",
            textAlign: "center",
            padding: "24px 16px",
            gap: "12px",
          }}
        >
          <Info
            size={32}
            strokeWidth={1.5}
            style={{ opacity: 0.4, marginBottom: "4px" }}
          />
          {selectedCount > 1 ? (
            <>
              <h3
                style={{
                  fontSize: "12px",
                  fontWeight: 600,
                  color: "var(--text)",
                  margin: 0,
                }}
              >
                {t("details.selectedCount", { count: selectedCount })}
              </h3>
              <p style={{ fontSize: "10px", margin: 0, lineHeight: 1.4 }}>
                {t("details.selectedCountDesc")}
              </p>
            </>
          ) : (
            <>
              <h3
                style={{
                  fontSize: "12px",
                  fontWeight: 600,
                  color: "var(--text)",
                  margin: 0,
                }}
              >
                {t("details.notSelected")}
              </h3>
              <p style={{ fontSize: "10px", margin: 0, lineHeight: 1.4 }}>
                {t("details.notSelectedDesc")}
              </p>
            </>
          )}
        </div>
      </aside>
    );
  }

  const action = async (value: string) => {
    try {
      await api.action(task.id, value);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  return (
    <aside className="details-pane">
      <div className="details-header">
        <h2>
          {task.file_name}
          {selectedCount > 1 && (
            <span
              style={{
                fontSize: "11px",
                fontWeight: 400,
                color: "var(--subtle)",
                marginLeft: "8px",
              }}
            >
              ({t("details.selectedCount", { count: selectedCount })})
            </span>
          )}
        </h2>
        <button onClick={onClose} title={t("common.close")}>
          <X size={14} />
        </button>
      </div>
      <div className="details-tabs" role="tablist" aria-label={t("details.title")}>
        <button
          role="tab"
          aria-selected={tab === "info"}
          className={tab === "info" ? "active" : ""}
          onClick={() => setTab("info")}
        >
          {t("details.tabInfo")}
        </button>
        <button
          role="tab"
          aria-selected={tab === "diagnosis"}
          className={tab === "diagnosis" ? "active" : ""}
          onClick={() => setTab("diagnosis")}
        >
          {t("details.tabDiagnosis")}
        </button>
        <button
          role="tab"
          aria-selected={tab === "precheck"}
          className={tab === "precheck" ? "active" : ""}
          onClick={() => setTab("precheck")}
        >
          {t("details.tabPrecheck")}
        </button>
        <button
          role="tab"
          aria-selected={tab === "connections"}
          className={tab === "connections" ? "active" : ""}
          onClick={() => setTab("connections")}
        >
          {t("details.tabConnections")}
        </button>
      </div>
      <div className="details-scroll">
        {tab === "info" && (
          <>
            <SpeedHistoryCard samples={speedHistory} />
            <DetailsInfoTab
              task={task}
              showMore={showMore}
              onToggleMore={() => setShowMore((v) => !v)}
              notify={notify}
              action={action}
              onTagsChanged={onTagsChanged}
            />
          </>
        )}
        {tab === "diagnosis" && (
          <DiagnosisPanel
            taskId={task.id}
            status={task.status}
            notify={notify}
            onOpenProxySettings={onOpenProxySettings}
            onOpenYouTubeModal={onOpenYouTubeModal}
            onOpenRefreshUrl={() => onOpenRefreshUrl?.(task)}
            onTaskChanged={() => {}}
          />
        )}
        {tab === "precheck" && (
          <PrecheckPanel
            result={precheck.result}
            loading={precheck.loading}
            error={precheck.error}
            onRefresh={() => void runPrecheck()}
            compact
          />
        )}
        {tab === "connections" &&
          (task.task_kind === "bt" ? (
            <BtDetailsPanel task={task} notify={notify} />
          ) : (
            <DetailsConnectionsTab task={task} />
          ))}
      </div>
    </aside>
  );
}
