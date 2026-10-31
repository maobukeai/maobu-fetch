import { useState, type MouseEvent } from "react";
import { Square } from "lucide-react";
import { api } from "../../api";
import { t } from "../../i18n";
import type { DownloadTask } from "../../types";
import { canFinishRecording } from "./recording-control";

export function FinishRecordingButton({ task, notify }: {
  task: DownloadTask;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [saving, setSaving] = useState(false);
  if (!canFinishRecording(task)) return null;
  const finish = async (event: MouseEvent) => {
    event.stopPropagation();
    if (saving) return;
    setSaving(true);
    try {
      await api.action(task.id, "finish-recording");
      notify(t("table.recordingSaved"));
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };
  return <button type="button" className="task-status-btn task-recording-finish"
    title={t("table.finishRecording")} aria-label={t("table.finishRecording")}
    disabled={saving} aria-busy={saving} onClick={(event) => void finish(event)}>
    <Square size={10} strokeWidth={2.5} />
  </button>;
}
