import { useEffect, useState } from "react";
import {
  AlertCircle, ChevronDown, ChevronRight, ClipboardCopy, FolderOpen,
  Info, LoaderCircle, RefreshCw, RotateCcw, Settings, ShieldCheck, Stethoscope,
} from "lucide-react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import type { ErrorDiagnosis, SuggestedAction, TaskStatus } from "../types";
import { api } from "../api";

/** 触发诊断面板自动加载的状态集合。 */
const TRIGGERING_STATUSES: TaskStatus[] = ["failed", "interrupted", "remote-changed", "paused-by-low-disk"];

const actionIcon: Record<SuggestedAction["action_id"], typeof RefreshCw> = {
  refetch_url: ClipboardCopy,
  clear_shards: RotateCcw,
  change_dir: FolderOpen,
  disable_proxy: Settings,
  reverify: ShieldCheck,
  retry: RefreshCw,
  redownload: RotateCcw,
};

export interface DiagnosisPanelProps {
  taskId: string;
  status: TaskStatus;
  notify: (text: string, kind?: "ok" | "error") => void;
  /** 跳转到设置页的代理部分（disable_proxy 动作）。 */
  onOpenProxySettings?: () => void;
  onOpenYouTubeModal?: () => void;
  /** 完成诊断后建议刷新任务列表，调用方传入。 */
  onTaskChanged?: () => void;
}

/**
 * 错误诊断面板：在任务详情中显示错误类别、中文说明、建议操作按钮和脱敏原始错误。
 *
 * 当任务进入 Failed / Interrupted / RemoteChanged / PausedByLowDisk 时自动调用
 * `api.diagnose(id)` 获取诊断结果。
 */
export function DiagnosisPanel({ taskId, status, notify, onOpenProxySettings, onOpenYouTubeModal, onTaskChanged }: DiagnosisPanelProps) {
  const [diagnosis, setDiagnosis] = useState<ErrorDiagnosis | null | undefined>(undefined);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [rawOpen, setRawOpen] = useState(false);
  const [busyAction, setBusyAction] = useState<string>();

  const shouldDiagnose = TRIGGERING_STATUSES.includes(status);

  useEffect(() => {
    setDiagnosis(undefined);
    setError(undefined);
    setRawOpen(false);
    if (!shouldDiagnose) return;
    let cancelled = false;
    setLoading(true);
    void api.diagnose(taskId)
      .then((result) => { if (!cancelled) setDiagnosis(result); })
      .catch((err) => { if (!cancelled) setError(String(err)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [taskId, shouldDiagnose]);

  if (!shouldDiagnose) {
    return (
      <div className="diagnosis-panel diagnosis-empty">
        <Stethoscope size={16} />
        <p>任务未进入错误状态，无需诊断。</p>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="diagnosis-panel diagnosis-loading">
        <LoaderCircle className="spin" size={14} />
        <span>正在诊断错误...</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="diagnosis-panel diagnosis-error">
        <AlertCircle size={14} />
        <span>诊断请求失败：{error}</span>
        <button type="button" className="diagnosis-retry" onClick={() => {
          setError(undefined);
          setLoading(true);
          void api.diagnose(taskId)
            .then(setDiagnosis)
            .catch((err) => setError(String(err)))
            .finally(() => setLoading(false));
        }}>重试</button>
      </div>
    );
  }

  if (!diagnosis) {
    return (
      <div className="diagnosis-panel diagnosis-empty">
        <Info size={14} />
        <span>未记录错误信息。可尝试重试或检查链接、登录态、网络。</span>
      </div>
    );
  }

  const runAction = async (action: SuggestedAction) => {
    if (busyAction) return;
    setBusyAction(action.action_id);
    try {
      switch (action.action_id) {
        case "refetch_url":
          notify("请从浏览器重新获取链接，粘贴后再次创建任务。");
          break;
        case "clear_shards":
          // 后端未提供独立 "clear-shards" 动作时回退到 redownload。
          try {
            await api.action(taskId, "clear-shards");
          } catch (e) {
            if (String(e).includes("Unknown action") || String(e).includes("未知动作")) {
              await api.action(taskId, "redownload");
            } else {
              throw e;
            }
          }
          notify("已清除旧分片，将重新下载。");
          onTaskChanged?.();
          break;
        case "change_dir": {
          const picked = await pickPath({ directory: true, multiple: false, title: "选择新的保存目录" });
          if (typeof picked !== "string") break;
          notify(`已选择目录：${picked}。请使用新目录重新创建任务，或暂停后修改保存位置。`);
          break;
        }
        case "disable_proxy":
          onOpenProxySettings?.();
          notify("已跳转到代理设置。");
          break;
        case "reverify": {
          const hash = await api.verify(taskId);
          notify(`校验完成：${hash.slice(0, 12)}…`);
          onTaskChanged?.();
          break;
        }
        case "retry":
          await api.action(taskId, "retry");
          notify("已重试任务。");
          onTaskChanged?.();
          break;
        case "redownload":
          await api.action(taskId, "redownload");
          notify("已保留旧文件并重新开始下载。");
          onTaskChanged?.();
          break;
      }
    } catch (err) {
      notify(String(err), "error");
    } finally {
      setBusyAction(undefined);
    }
  };

  return (
    <div className="diagnosis-panel">
      <div className="diagnosis-header">
        <AlertCircle size={14} />
        <strong>{diagnosis.title}</strong>
      </div>
      <p className="diagnosis-description">{diagnosis.description}</p>

      <div className="diagnosis-actions">
        {diagnosis.suggested_actions.map((action, idx) => {
          const Icon = actionIcon[action.action_id] ?? RefreshCw;
          const isBusy = busyAction === action.action_id;
          return (
            <button
              key={`${action.action_id}-${idx}`}
              type="button"
              className="diagnosis-action-btn"
              disabled={Boolean(busyAction)}
              onClick={() => void runAction(action)}
            >
              {isBusy ? <LoaderCircle className="spin" size={12} /> : <Icon size={12} />}
              <span>{action.label}</span>
            </button>
          );
        })}
        {diagnosis.description.includes("YouTube") && (
          <button
            type="button"
            className="diagnosis-action-btn"
            onClick={() => {
              if (onOpenYouTubeModal) {
                onOpenYouTubeModal();
              } else {
                void api.openExternalUrl("https://www.youtube.com");
              }
            }}
          >
            <ShieldCheck size={12} />
            <span>解决 YouTube 凭证与验证</span>
          </button>
        )}
      </div>

      <button
        type="button"
        className={`diagnosis-raw-toggle ${rawOpen ? "open" : ""}`}
        onClick={() => setRawOpen((v) => !v)}
        aria-expanded={rawOpen}
      >
        {rawOpen ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
        <span>原始错误（已脱敏）</span>
      </button>
      {rawOpen && (
        <pre className="diagnosis-raw-text">{diagnosis.raw_error_redacted || "（无原始错误文本）"}</pre>
      )}
    </div>
  );
}
