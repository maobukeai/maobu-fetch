import { useEffect, useState } from "react";
import { api } from "../../api";
import type { DownloadTask, RetryPolicy } from "../../types";
import { RetryPolicyEditor } from "./RetryPolicyEditor";

export function TaskRetryPolicySection({
  task,
  notify,
}: {
  task: DownloadTask;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const hasOverride = task.retry_policy_override != null;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<RetryPolicy | null>(
    task.retry_policy_override ?? null
  );
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDraft(task.retry_policy_override ?? null);
    setEditing(false);
  }, [task.id, task.retry_policy_override]);

  const backoffLabel = (policy: RetryPolicy | null | undefined) => {
    if (!policy) return "全局默认";
    return policy.backoff === "exponential" ? "指数退避" : "固定间隔";
  };
  const summary = (policy: RetryPolicy | null | undefined) => {
    if (!policy) return "使用全局默认策略";
    const timeout =
      policy.task_timeout_secs == null
        ? "无总超时"
        : `总超时 ${policy.task_timeout_secs}s`;
    return `${policy.connection_timeout_secs}s 连接超时 · ${timeout} · ${policy.max_retries} 次重试 · ${backoffLabel(policy)}`;
  };

  const startEdit = () => {
    setDraft(
      task.retry_policy_override ?? {
        connection_timeout_secs: 60,
        task_timeout_secs: null,
        max_retries: 5,
        backoff: "exponential",
        initial_backoff_ms: 1000,
        max_backoff_ms: 60000,
      }
    );
    setEditing(true);
  };

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      await api.updateRetryPolicy(task.id, draft);
      notify("任务重试策略已保存");
      setEditing(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const clearOverride = async () => {
    setSaving(true);
    try {
      await api.updateRetryPolicy(task.id, null);
      notify("已恢复使用全局默认重试策略");
      setEditing(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="task-retry-policy-section">
      <div className="task-retry-policy-header">
        <strong>重试策略</strong>
        <span className="task-retry-policy-summary">
          {editing
            ? "✏️ 正在编辑自定义重试策略"
            : summary(task.retry_policy_override)}
        </span>
      </div>
      {!editing && (
        <div className="task-retry-policy-actions">
          <button onClick={startEdit} disabled={saving}>
            自定义覆盖
          </button>
          {hasOverride && (
            <button onClick={() => void clearOverride()} disabled={saving}>
              恢复全局默认
            </button>
          )}
        </div>
      )}
      {editing && draft && (
        <div className="task-retry-policy-editor retry-policy-grid">
          <RetryPolicyEditor
            value={draft}
            onChange={setDraft}
            disabled={saving}
            compact
          />
          <div className="task-retry-policy-actions">
            <button
              className="primary"
              onClick={() => void save()}
              disabled={saving}
            >
              {saving ? "保存中…" : "保存覆盖"}
            </button>
            <button
              onClick={() => {
                setEditing(false);
                setDraft(task.retry_policy_override ?? null);
              }}
              disabled={saving}
            >
              取消
            </button>
            {hasOverride && (
              <button
                onClick={() => void clearOverride()}
                disabled={saving}
              >
                清除覆盖
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
