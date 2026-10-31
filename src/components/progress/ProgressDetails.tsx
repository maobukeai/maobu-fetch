import { ChevronDown, ChevronUp, ExternalLink, Pin, PinOff, X } from "lucide-react";
import { formatBytes } from "../../formatters";
import { t } from "../../i18n";
import { progressApi, type ProgressWindowSnapshot } from "./progress-api";
import { EXPANDED_ROWS, nonNegative, visibleTasks } from "./progress-model";
import { ProgressTaskRow } from "./ProgressTaskRow";

type Operation = (key: string, action: () => Promise<ProgressWindowSnapshot>) => Promise<void>;

export function ProgressDetails({ snapshot, error, busy, perform, reconnect, onDismiss }: {
  snapshot: ProgressWindowSnapshot | null;
  error: string;
  busy: string | null;
  perform: Operation;
  reconnect: () => void;
  onDismiss: () => void;
}) {
  const pinned = snapshot?.always_on_top ?? true;
  const count = snapshot?.total_count ?? 0;
  const expanded = snapshot?.expanded ?? false;
  const loading = !snapshot || snapshot.loading;
  const rows = snapshot ? visibleTasks(snapshot) : [];
  const pinTitle = t(pinned ? "progressWindow.unpin" : "progressWindow.pin");
  const openMain = (id?: string) => { void perform("main", () => progressApi.showMain(id)); };
  return <section id="progress-window-details" className="progress-window-details" aria-label={t("progressWindow.taskList")}>
    <header className="progress-window-header">
      <span className="progress-window-heading">{t("progressWindow.title")}</span>
      <span className="progress-window-total-speed" title={t("progressWindow.totalSpeed")}>{formatBytes(nonNegative(snapshot?.total_speed_bps ?? 0))}/s</span>
      <button type="button" className={`progress-window-icon${pinned ? " is-pinned" : ""}`}
        title={pinTitle} aria-label={pinTitle} aria-pressed={pinned} disabled={Boolean(busy) || !snapshot}
        onClick={() => { void perform("pin", () => progressApi.pinned(!pinned)); }}>
        {pinned ? <Pin size={12} /> : <PinOff size={12} />}
      </button>
      <button type="button" className="progress-window-icon" title={t("progressWindow.hide")}
        aria-label={t("progressWindow.hide")} disabled={Boolean(busy)} onClick={onDismiss}><X size={13} /></button>
    </header>
    {(error || snapshot?.error) && <div className="progress-window-error" role="alert">
      <span>{error || t("progressWindow.connectionError")}</span>
      <button type="button" disabled={Boolean(busy)} onClick={reconnect}>{t("common.retry")}</button>
    </div>}
    <ul className="progress-window-tasks" aria-label={t("progressWindow.taskList")}>
      {rows.map(task => <ProgressTaskRow key={task.id} task={task} disabled={Boolean(busy)}
        onOpen={() => openMain(task.id)} onAction={() => { void perform(`pause:${task.id}`, () => progressApi.action(task.id, "pause")); }} />)}
      {rows.length === 0 && <li className="progress-window-empty" role="status">{t(loading ? "progressWindow.loading" : "progressWindow.empty")}</li>}
    </ul>
    <footer className="progress-window-footer">
      {count > 3 ? <button type="button" className="progress-window-fold" aria-expanded={expanded}
        disabled={Boolean(busy)} onClick={() => { void perform("expand", () => progressApi.expanded(!expanded)); }}>
        {expanded ? <ChevronUp size={11} /> : <ChevronDown size={11} />}
        {t(expanded ? "progressWindow.compactCollapse" : "progressWindow.compactExpand")}
      </button> : <span className="progress-window-footer-hint">{t("progressWindow.taskCount", { count })}</span>}
      <button type="button" className="progress-window-open" onClick={() => openMain()} disabled={Boolean(busy)}>
        {t(count > EXPANDED_ROWS ? "progressWindow.viewAll" : "progressWindow.openMain", { count })}<ExternalLink size={11} />
      </button>
    </footer>
  </section>;
}
