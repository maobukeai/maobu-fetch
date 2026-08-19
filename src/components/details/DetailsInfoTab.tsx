import { useEffect, useState } from "react";
import {
  AlertCircle,
  Bookmark,
  CirclePause,
  Clock,
  Copy,
  ChevronDown,
  FolderOpen,
  Pause,
  Play,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";
import { api, isDesktop } from "../../api";
import { listen } from "@tauri-apps/api/event";
import { t, useLocale } from "../../i18n";
import type { DownloadTask, TaskTemplate, WaitReason } from "../../types";
import {
  clampPriority,
  formatBytes,
  formatDuration,
  getStatusText,
  hostOf,
  MIN_PRIORITY,
  MAX_PRIORITY,
  PRIORITY_STEP,
  redactedUrl,
  waitReasonText,
} from "../../formatters";
import { completionActionLabel } from "../CompletionActionEditor";
import { Select } from "../Select";
import { Modal } from "../common/Modal";
import { Field } from "../common/FormComponents";
import { isMediaTask, btRuntimeActive } from "../common/TaskRow";
import { TaskRetryPolicySection } from "./TaskRetryPolicySection";
import { TaskProxySection } from "./TaskProxySection";
import { TaskTagEditor } from "./TaskTagEditor";

export function DetailValue({
  label,
  value,
  notify,
}: {
  label: string;
  value: string;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  return (
    <div>
      <dt>{label}</dt>
      <dd className="detail-copy-value" title={value}>
        <span>{value}</span>
        <button
          onClick={() =>
            void navigator.clipboard
              .writeText(value)
              .then(() => notify(t("details.copied", { label })))
              .catch((error) => notify(String(error), "error"))
          }
          title={t("details.copyLabel", { label })}
        >
          <Copy size={11} />
        </button>
      </dd>
    </div>
  );
}

function BtInfoRows({
  task,
  notify,
}: {
  task: DownloadTask;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const runtime = task.bt_runtime;
  const uploaded = runtime?.uploaded_bytes ?? 0;
  const ratio = task.total_bytes > 0 ? uploaded / task.total_bytes : null;
  const seedingNow = btRuntimeActive(task) && (runtime?.seeding ?? false);
  return (
    <>
      <div>
        <dt>{t("details.taskKind")}</dt>
        <dd>{t("details.btTaskKindValue")}</dd>
      </div>
      {task.bt_meta?.info_hash && (
        <DetailValue
          label={t("details.btInfoHash")}
          value={task.bt_meta.info_hash}
          notify={notify}
        />
      )}
      <div>
        <dt>{t("details.btSeedingState")}</dt>
        <dd>{seedingNow ? t("table.seeding") : t("details.btNotSeeding")}</dd>
      </div>
      <div>
        <dt>{t("details.btUploaded")}</dt>
        <dd>{uploaded > 0 ? formatBytes(uploaded) : "—"}</dd>
      </div>
      <div>
        <dt>{t("details.btShareRatio")}</dt>
        <dd>{ratio !== null ? `${ratio.toFixed(2)}` : "—"}</dd>
      </div>
    </>
  );
}

export function DetailsInfoTab({
  task,
  showMore,
  onToggleMore,
  notify,
  action,
  onTagsChanged,
}: {
  task: DownloadTask;
  showMore: boolean;
  onToggleMore: () => void;
  notify: (text: string, kind?: "ok" | "error") => void;
  action: (value: string) => Promise<void>;
  onTagsChanged?: () => void;
}) {
  useLocale();
  const statusText = getStatusText();
  const [waitReason, setWaitReason] = useState<WaitReason | null>(null);
  const [priorityInput, setPriorityInput] = useState(String(task.priority));
  const [prioritySaving, setPrioritySaving] = useState(false);
  const taskId = task.id;
  const taskStatus = task.status;
  const isWaiting = taskStatus === "queued" || taskStatus === "scheduled";

  useEffect(() => {
    if (!isWaiting) {
      setWaitReason(null);
      return;
    }

    let cancelled = false;
    const unlistens: Array<() => void> = [];

    const fetchReason = () => {
      if (cancelled) return;
      api
        .getWaitReason(taskId)
        .then((reason) => {
          if (!cancelled) setWaitReason(reason);
        })
        .catch(() => {
          if (!cancelled) setWaitReason(null);
        });
    };

    fetchReason();

    if (isDesktop()) {
      Promise.all([
        listen<DownloadTask>("task-updated", fetchReason),
        listen<DownloadTask>("task-created", fetchReason),
        listen<string>("task-removed", fetchReason),
      ]).then((fns) => {
        if (cancelled) {
          fns.forEach((fn) => fn());
        } else {
          unlistens.push(...fns);
        }
      });
    }

    return () => {
      cancelled = true;
      unlistens.forEach((fn) => fn());
    };
  }, [taskId, taskStatus, isWaiting]);

  useEffect(() => {
    setPriorityInput(String(task.priority));
  }, [task.id, task.priority]);

  const commitPriority = async () => {
    if (prioritySaving) return;
    const trimmed = priorityInput.trim();
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) {
      notify(t("toasts.priorityMustBeInteger"), "error");
      setPriorityInput(String(task.priority));
      return;
    }
    const clamped = clampPriority(parsed);
    if (clamped !== parsed) {
      notify(
        t("toasts.priorityClamped", { min: MIN_PRIORITY, max: MAX_PRIORITY }),
        "error"
      );
    }
    if (clamped === task.priority) {
      setPriorityInput(String(clamped));
      return;
    }
    setPrioritySaving(true);
    try {
      await api.updateTaskOptions(task.id, { priority: clamped });
      notify(t("toasts.priorityUpdated"));
    } catch (error) {
      notify(String(error), "error");
      setPriorityInput(String(task.priority));
    } finally {
      setPrioritySaving(false);
    }
  };

  const completionLabel = completionActionLabel(task.completion_action);
  const priorityLabel =
    task.priority < 0
      ? t("details.priorityHigh")
      : task.priority > 0
      ? t("details.priorityLow")
      : t("details.priorityNormal");
  const waitReasonLabel = waitReason ? waitReasonText(waitReason) : null;
  const hasTempAuth =
    !!task.headers &&
    Object.keys(task.headers).some((name) => {
      const lower = name.toLowerCase();
      return (
        lower === "cookie" ||
        lower === "referer" ||
        lower === "referrer" ||
        lower === "user-agent"
      );
    });

  const [saveTplOpen, setSaveTplOpen] = useState(false);
  const [tplDraft, setTplDraft] = useState<TaskTemplate | null>(null);
  const [tplHeadersText, setTplHeadersText] = useState("");

  const openSaveAsTemplate = () => {
    let domain = "";
    try {
      domain = new URL(task.url).hostname.toLowerCase();
    } catch {
      domain = "";
    }
    const headers =
      task.headers && Object.keys(task.headers).length > 0
        ? { ...task.headers }
        : null;
    setTplDraft({
      id: `tpl-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: domain ? `${domain} ${t("common.custom")}` : t("common.custom"),
      domain_pattern: domain,
      connections: task.connection_count,
      speed_limit: task.per_task_speed_limit || null,
      headers,
      destination: task.destination || null,
      completion_action:
        task.completion_action === "none" ? null : task.completion_action,
      enabled: true,
      priority: 0,
    });
    setTplHeadersText(
      headers
        ? Object.entries(headers)
            .map(([k, v]) => `${k}: ${v}`)
            .join("\n")
        : ""
    );
    setSaveTplOpen(true);
  };

  const saveAsTemplate = async () => {
    if (!tplDraft) return;
    if (!tplDraft.name.trim()) {
      notify(t("toasts.tagNameEmpty"), "error");
      return;
    }
    if (!tplDraft.domain_pattern.trim()) {
      notify(t("toasts.tagNameEmpty"), "error");
      return;
    }
    let headers: Record<string, string> | null = null;
    const trimmedHeaders = tplHeadersText.trim();
    if (trimmedHeaders) {
      headers = {};
      for (const line of trimmedHeaders.split(/\r?\n/)) {
        const lineTrim = line.trim();
        if (!lineTrim) continue;
        const idx = lineTrim.indexOf(":");
        if (idx <= 0) {
          notify(String(line), "error");
          return;
        }
        headers[lineTrim.slice(0, idx).trim()] = lineTrim.slice(idx + 1).trim();
      }
      if (Object.keys(headers).length === 0) headers = null;
    }
    const toSave: TaskTemplate = {
      ...tplDraft,
      headers,
      destination: tplDraft.destination?.trim() || null,
    };
    try {
      await api.taskTemplateAdd(toSave);
      notify(t("toasts.settingsSaved"));
      setSaveTplOpen(false);
      setTplDraft(null);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  return (
    <>
      <dl>
        <div>
          <dt>{t("details.status")}</dt>
          <dd>
            {task.status === "downloading" &&
            isMediaTask(task) &&
            task.downloaded_bytes === 0 &&
            task.active_connections === 0 &&
            !task.error
              ? statusText.parsing
              : statusText[task.status]}
          </dd>
        </div>
        <div>
          <dt>{t("details.size")}</dt>
          <dd>{task.total_bytes ? formatBytes(task.total_bytes) : "—"}</dd>
        </div>
        <div>
          <dt>{t("details.speed")}</dt>
          <dd>{task.speed ? `${formatBytes(task.speed)}/s` : "—"}</dd>
        </div>
        <div>
          <dt>{t("details.eta")}</dt>
          <dd>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</dd>
        </div>
        <div>
          <dt>{t("details.sourceDomain")}</dt>
          <dd>{hostOf(task.url)}</dd>
        </div>
        <div>
          <dt>{t("details.saveLocation")}</dt>
          <dd>{task.destination}</dd>
        </div>
        <div>
          <dt>{t("details.priority")}</dt>
          <dd>
            <input
              type="number"
              value={priorityInput}
              min={MIN_PRIORITY}
              max={MAX_PRIORITY}
              step={PRIORITY_STEP}
              disabled={prioritySaving}
              onChange={(e) => setPriorityInput(e.target.value)}
              onBlur={() => void commitPriority()}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
              }}
              style={{ width: "88px" }}
              aria-label={t("details.priority")}
            />
            <span style={{ marginLeft: "6px" }} title={t("details.priorityHint")}>
              {priorityLabel}
            </span>
            <small
              style={{
                display: "block",
                marginTop: "2px",
                fontSize: "10px",
                opacity: 0.7,
                whiteSpace: "nowrap",
              }}
            >
              {t("details.priorityRange", {
                min: MIN_PRIORITY,
                max: MAX_PRIORITY,
              })}
            </small>
          </dd>
        </div>
        <div>
          <dt>{t("details.taskSpeedLimit")}</dt>
          <dd>
            {task.per_task_speed_limit
              ? `${Math.round(task.per_task_speed_limit / 1024)} KB/s`
              : t("details.noSpeedLimit")}
          </dd>
        </div>
        <div>
          <dt>{t("details.completionAction")}</dt>
          <dd>{completionLabel}</dd>
        </div>
        <div>
          <dt>{t("details.downloadSource")}</dt>
          <dd>{task.source}</dd>
        </div>
        {task.task_kind === "bt" && <BtInfoRows task={task} notify={notify} />}
      </dl>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "6px",
          marginTop: "-4px",
        }}
      >
        {hasTempAuth && (
          <div
            className="temp-auth-banner"
            role="status"
            title={t("details.tempAuthHint")}
          >
            <ShieldCheck size={13} />
            <span>{t("details.tempAuthBadge")}</span>
          </div>
        )}

        {waitReasonLabel && (
          <div className="wait-reason-banner" role="status">
            <Clock size={13} />
            <span>{waitReasonLabel}</span>
          </div>
        )}

        <button
          className={`details-more-toggle ${showMore ? "open" : ""}`}
          onClick={onToggleMore}
        >
          <ChevronDown size={11} />
          {t("details.moreInfo")}
        </button>

        {showMore && (
          <dl>
            <DetailValue
              label={t("details.originalUrl")}
              value={redactedUrl(task.url)}
              notify={notify}
            />
            {task.final_url && (
              <DetailValue
                label={t("details.finalUrl")}
                value={task.final_url}
                notify={notify}
              />
            )}
            {task.response_status && (
              <div>
                <dt>{t("details.httpStatus")}</dt>
                <dd>{task.response_status}</dd>
              </div>
            )}
            {task.content_type && (
              <DetailValue
                label={t("details.contentType")}
                value={task.content_type}
                notify={notify}
              />
            )}
            {task.accepts_ranges !== undefined && (
              <div>
                <dt>{t("details.acceptsRanges")}</dt>
                <dd>
                  {task.accepts_ranges
                    ? t("details.rangeSupported")
                    : t("details.rangeNotSupported")}
                </dd>
              </div>
            )}
            {task.etag && (
              <DetailValue label="ETag" value={task.etag} notify={notify} />
            )}
            {task.last_modified && (
              <DetailValue
                label="Last-Modified"
                value={task.last_modified}
                notify={notify}
              />
            )}
            <div>
              <dt>{t("details.retryCount")}</dt>
              <dd>
                {task.retry_count} / {task.max_retries}
              </dd>
            </div>
            {task.checksum_sha256 && (
              <div>
                <dt>{t("details.sha256")}</dt>
                <dd title={task.checksum_sha256}>
                  {task.checksum_sha256.slice(0, 16)}…
                </dd>
              </div>
            )}
          </dl>
        )}

        <p className="details-security-note" style={{ margin: 0 }}>
          {t("details.securityNote")}
        </p>
      </div>

      {task.error && <div className="task-error">{task.error}</div>}

      {task.status === "remote-changed" && (
        <div className="remote-changed-banner" role="alert">
          <AlertCircle size={16} />
          <div className="remote-changed-body">
            <strong>{t("details.remoteChanged")}</strong>
            <p>{t("details.remoteChangedDesc")}</p>
            <div className="remote-changed-actions">
              <button
                className="remote-changed-redownload"
                onClick={() =>
                  void action("redownload").then(() =>
                    notify(t("toasts.redownloading"))
                  )
                }
                title={t("details.redownloadHint")}
              >
                <RefreshCw size={13} />
                {t("details.redownloadAction")}
              </button>
              <button
                className="remote-changed-keep"
                onClick={() => void action("cancel")}
                title={t("details.keepOldFileHint")}
              >
                <CirclePause size={13} />
                {t("details.keepOldFile")}
              </button>
            </div>
          </div>
        </div>
      )}

      {task.status === "paused-by-metered" && (
        <div className="remote-changed-banner" role="status">
          <AlertCircle size={16} />
          <div className="remote-changed-body">
            <strong>{t("details.meteredPaused")}</strong>
            <p>{t("details.meteredPausedDesc")}</p>
            <div className="remote-changed-actions">
              <button
                className="remote-changed-redownload"
                onClick={() =>
                  void action("resume").then(() =>
                    notify(t("toasts.meteredResumed"))
                  )
                }
                title={t("details.resumeDownloadHint")}
              >
                <Play size={13} />
                {t("details.resumeDownload")}
              </button>
            </div>
          </div>
        </div>
      )}

      {task.segments.length > 0 && (
        <div className="segment-panel">
          <div className="segment-title">
            {t("details.segments", {
              active: task.active_connections,
              max: task.connection_count,
              count: task.segments.length,
            })}
          </div>
          <div className="segment-list">
            {task.segments.map((segment) => {
              const size = segment.end_byte - segment.start_byte + 1;
              const value = size
                ? Math.min(100, (segment.downloaded_bytes / size) * 100)
                : 0;
              return (
                <div
                  className={`segment-item ${
                    segment.status === "downloading" &&
                    task.status === "downloading"
                      ? "active"
                      : ""
                  }`}
                  key={segment.index}
                >
                  <span>#{segment.index + 1}</span>
                  <div>
                    <i style={{ width: `${value}%` }} />
                  </div>
                  <em>{value.toFixed(0)}%</em>
                </div>
              );
            })}
          </div>
        </div>
      )}

      <div className="details-actions">
        {["downloading", "waiting-network"].includes(task.status) ? (
          <button onClick={() => void action("pause")}>
            <Pause size={13} />
            {t("details.pauseDownload")}
          </button>
        ) : (
          !["completed", "cancelled", "remote-changed"].includes(task.status) && (
            <button onClick={() => void action("resume")}>
              <Play size={13} />
              {t("details.resumeDownload")}
            </button>
          )
        )}
        <button
          onClick={() =>
            void api
              .openFolder(task.id)
              .catch((error) => notify(String(error), "error"))
          }
        >
          <FolderOpen size={13} />
          {t("details.openDirectory")}
        </button>
        {task.status === "completed" && (
          <button
            onClick={async () => {
              try {
                const hash = await api.verify(task.id);
                notify(t("toasts.verifyComplete", { hash: hash.slice(0, 12) }));
              } catch (error) {
                notify(String(error), "error");
              }
            }}
          >
            <ShieldCheck size={13} />
            {t("details.verifyFile")}
          </button>
        )}
        <button
          onClick={openSaveAsTemplate}
          title={t("details.saveAsTemplate")}
        >
          <Bookmark size={13} />
          {t("details.saveAsTemplate")}
        </button>
      </div>

      <TaskRetryPolicySection task={task} notify={notify} />
      <TaskProxySection task={task} notify={notify} />
      <TaskTagEditor
        task={task}
        notify={notify}
        onTagsChanged={onTagsChanged}
      />

      {saveTplOpen && tplDraft && (
        <Modal
          title="保存为任务模板"
          onClose={() => setSaveTplOpen(false)}
          style={{ width: "520px" }}
        >
          <div className="category-rule-edit-form">
            <p className="settings-note" style={{ margin: "0 0 4px" }}>
              将当前任务的下载参数保存为模板，下次新建同域名任务时自动套用到未显式设置的字段。
            </p>
            <div className="template-edit-grid">
              <Field label="模板名称">
                <input
                  value={tplDraft.name}
                  onChange={(e) =>
                    setTplDraft({ ...tplDraft, name: e.target.value })
                  }
                />
              </Field>
              <Field label="域名匹配模式">
                <input
                  value={tplDraft.domain_pattern}
                  onChange={(e) =>
                    setTplDraft({
                      ...tplDraft,
                      domain_pattern: e.target.value,
                    })
                  }
                  placeholder="github.com 或 *.github.com"
                />
              </Field>
              <Field label="连接数（留空表示不覆盖；仅允许 1 / 2 / 4 / 8 / 16 / 32）">
                <Select
                  value={tplDraft.connections ?? ""}
                  onChange={(val: any) => {
                    setTplDraft({
                      ...tplDraft,
                      connections: val === "" ? null : +val,
                    });
                  }}
                  options={[
                    { value: "", label: "不覆盖" },
                    { value: 1, label: "1 路" },
                    { value: 2, label: "2 路" },
                    { value: 4, label: "4 路" },
                    { value: 8, label: "8 路" },
                    { value: 16, label: "16 路" },
                    { value: 32, label: "32 路" },
                  ]}
                  ariaLabel="连接数"
                />
              </Field>
              <Field label="单任务限速（KB/s，0 或留空表示不限速）">
                <input
                  type="number"
                  min="0"
                  value={
                    tplDraft.speed_limit
                      ? Math.round(tplDraft.speed_limit / 1024)
                      : 0
                  }
                  onChange={(e) => {
                    const v = +e.target.value;
                    setTplDraft({
                      ...tplDraft,
                      speed_limit: v > 0 ? v * 1024 : null,
                    });
                  }}
                />
              </Field>
              <Field
                className="wide"
                label="保存目录（留空表示不覆盖）"
              >
                <input
                  value={tplDraft.destination ?? ""}
                  onChange={(e) =>
                    setTplDraft({
                      ...tplDraft,
                      destination: e.target.value || null,
                    })
                  }
                  placeholder="例如：D:\\Downloads\\GitHub"
                />
              </Field>
              <Field
                className="wide"
                label="请求头（每行一个，格式 Key: Value；留空表示不覆盖）"
              >
                <textarea
                  rows={3}
                  value={tplHeadersText}
                  onChange={(e) => setTplHeadersText(e.target.value)}
                  placeholder={
                    "Authorization: Bearer token\nUser-Agent: MaobuFetch"
                  }
                  style={{ width: "100%", fontFamily: "monospace" }}
                />
              </Field>
            </div>
            <div className="dialog-actions">
              <button onClick={() => setSaveTplOpen(false)}>取消</button>
              <button
                className="primary"
                onClick={() => void saveAsTemplate()}
              >
                保存
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}
