import { useEffect, useState } from "react";
import { ChevronDown, ChevronUp, ExternalLink, Pin, PinOff, X } from "lucide-react";
import { formatBytes } from "../../formatters";
import { t } from "../../i18n";
import { progressApi, type ProgressOperation, type ProgressWindowSnapshot } from "./progress-api";
import { completedCount, EXPANDED_ROWS, nonNegative, visibleCompleted, visibleTasks } from "./progress-model";
import { ProgressTaskRow } from "./ProgressTaskRow";
import { CompletedTaskRow } from "./CompletedTaskRow";
import { ProgressTabs, type ProgressTab } from "./ProgressTabs";

export function ProgressDetails({ snapshot, error, busy, perform, reconnect, onDismiss }: {
  snapshot: ProgressWindowSnapshot | null;
  error: string;
  busy: string | null;
  perform: ProgressOperation;
  reconnect: () => void;
  onDismiss: () => void;
}) {
  const pinned = snapshot?.always_on_top ?? true;
  const activeCount = snapshot?.total_count ?? 0;
  const completions = completedCount(snapshot);
  const [selection, setSelection] = useState<ProgressTab>("active");
  const tab = activeCount === 0 && completions > 0 ? "completed" : completions === 0 ? "active" : selection;
  const mixed = activeCount > 0 && completions > 0;
  const completedOnly = activeCount === 0 && completions > 0;
  const count = tab === "completed" ? completions : activeCount;
  useEffect(() => { if (!completions) setSelection("active"); }, [completions]);
  const expanded = snapshot?.expanded ?? false;
  const loading = !snapshot || snapshot.loading;
  const rows = snapshot ? (tab === "completed" ? visibleCompleted(snapshot) : visibleTasks(snapshot)) : [];
  const singleError = completedOnly && count === 1 && Boolean(error || snapshot?.error);
  const pinTitle = t(pinned ? "progressWindow.unpin" : "progressWindow.pin");
  const openMain = (id?: string) => { void perform("main", () => progressApi.showMain(id)); };
  const fold = () => { void perform("expand", () => progressApi.expanded(!expanded)); };
  return <section id="progress-window-details" className={`progress-window-details${completedOnly ? " is-completed-only" : ""}${singleError ? " has-single-error" : ""}`} aria-label={t(tab === "completed" ? "progressWindow.completedList" : "progressWindow.taskList")}>
    <header className="progress-window-header">
      <span className="progress-window-heading">{t(completedOnly ? "progressWindow.recentTitle" : "progressWindow.title")}</span>
      {completedOnly && count > 3 && <button type="button" className="progress-window-icon progress-window-fold progress-window-header-fold"
        aria-expanded={expanded} disabled={Boolean(busy)} title={t(expanded ? "progressWindow.compactCollapse" : "progressWindow.compactExpand")}
        aria-label={t(expanded ? "progressWindow.compactCollapse" : "progressWindow.compactExpand")} onClick={fold}>
        {expanded ? <ChevronUp size={12} /> : <ChevronDown size={12} />}
      </button>}
      {activeCount > 0 && <span className="progress-window-total-speed" title={t("progressWindow.totalSpeed")}>{formatBytes(nonNegative(snapshot?.total_speed_bps ?? 0))}/s</span>}
      <button type="button" className={`progress-window-icon${pinned ? " is-pinned" : ""}`}
        title={pinTitle} aria-label={pinTitle} aria-pressed={pinned} disabled={Boolean(busy) || !snapshot}
        onClick={() => { void perform("pin", () => progressApi.pinned(!pinned)); }}>
        {pinned ? <Pin size={12} /> : <PinOff size={12} />}
      </button>
      {completedOnly ? <button type="button" className="progress-window-icon progress-window-open" title={t("progressWindow.openMain")}
        aria-label={t("progressWindow.openMain")} disabled={Boolean(busy)} onClick={() => openMain()}><ExternalLink size={12} /></button>
        : <button type="button" className="progress-window-icon" title={t("progressWindow.hide")}
          aria-label={t("progressWindow.hide")} disabled={Boolean(busy)} onClick={onDismiss}><X size={13} /></button>}
    </header>
    {mixed && <ProgressTabs activeCount={activeCount} completedCount={completions} selected={tab} onSelect={setSelection} />}
    {(error || snapshot?.error) && <div className="progress-window-error" role="alert">
      <span title={error || t("progressWindow.connectionError")}>{error || t("progressWindow.connectionError")}</span>
      <button type="button" disabled={Boolean(busy)} onClick={reconnect}>{t("common.retry")}</button>
    </div>}
    {!singleError && <div className="progress-window-list-panel" role={mixed ? "tabpanel" : undefined} id={`progress-panel-${tab}`} aria-labelledby={mixed ? `progress-tab-${tab}` : undefined}>
    <ul className="progress-window-tasks" aria-label={t(tab === "completed" ? "progressWindow.completedList" : "progressWindow.taskList")}>
      {rows.map(task => tab === "completed"
        ? <CompletedTaskRow key={task.id} task={task} disabled={Boolean(busy)} showActions={!completedOnly || count > 1} onOpen={target => {
          void perform(`open:${task.id}:${target}`, () => progressApi.openCompleted(task.id, target), "progressWindow.openError");
        }} />
        : <ProgressTaskRow key={task.id} task={task} disabled={Boolean(busy)}
          onOpen={() => openMain(task.id)} onAction={() => { void perform(`pause:${task.id}`, () => progressApi.action(task.id, "pause")); }} />)}
      {rows.length === 0 && <li className="progress-window-empty" role="status">{t(loading ? "progressWindow.loading" : "progressWindow.empty")}</li>}
    </ul>
    </div>}
    {!completedOnly && <footer className="progress-window-footer">
      {count > 3 ? <button type="button" className="progress-window-fold" aria-expanded={expanded}
        disabled={Boolean(busy)} onClick={fold}>
        {expanded ? <ChevronUp size={11} /> : <ChevronDown size={11} />}
        {t(expanded ? "progressWindow.compactCollapse" : "progressWindow.compactExpand")}
      </button> : <span className="progress-window-footer-hint">{t(tab === "completed" ? "progressWindow.completedCount" : "progressWindow.taskCount", { count })}</span>}
      <button type="button" className="progress-window-open" onClick={() => openMain()} disabled={Boolean(busy)}>
        {t(count > EXPANDED_ROWS ? "progressWindow.viewAll" : "progressWindow.openMain", { count })}<ExternalLink size={11} />
      </button>
    </footer>}
  </section>;
}
