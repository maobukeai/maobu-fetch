import { useRef } from "react";
import { t } from "../../i18n";

export type ProgressTab = "active" | "completed";

export function ProgressTabs({ activeCount, completedCount, selected, onSelect }: {
  activeCount: number; completedCount: number; selected: ProgressTab; onSelect: (tab: ProgressTab) => void;
}) {
  const active = useRef<HTMLButtonElement>(null), completed = useRef<HTMLButtonElement>(null);
  return <div className="progress-window-tabs" role="tablist" aria-label={t("progressWindow.taskList")}>
    {(["active", "completed"] as const).map(tab => <button key={tab} type="button" role="tab"
      ref={tab === "active" ? active : completed} id={`progress-tab-${tab}`} data-view={tab}
      aria-selected={selected === tab} aria-controls={`progress-panel-${tab}`} tabIndex={selected === tab ? 0 : -1}
      onClick={() => onSelect(tab)} onKeyDown={event => {
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        const next = event.key === "Home" ? "active" : event.key === "End" ? "completed" : tab === "active" ? "completed" : "active";
        onSelect(next); (next === "active" ? active : completed).current?.focus();
      }}>{t(tab === "active" ? "progressWindow.activeTab" : "progressWindow.completedTab", { count: tab === "active" ? activeCount : completedCount })}</button>)}
  </div>;
}
