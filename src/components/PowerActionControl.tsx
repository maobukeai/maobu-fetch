import { useState } from "react";
import { Moon, Power, X } from "lucide-react";
import type { PowerAction, PowerActionState } from "../types";

const actionText: Record<PowerAction, string> = {
  none: "不执行操作",
  shutdown: "关机",
  hibernate: "休眠",
};

interface Props {
  state: PowerActionState;
  onArm: (action: Exclude<PowerAction, "none">) => Promise<void>;
  onCancel: () => Promise<void>;
}

export function PowerActionButton({ state, onArm, onCancel }: Props) {
  const [open, setOpen] = useState(false);
  const arm = async (action: Exclude<PowerAction, "none">) => {
    try {
      await onArm(action);
      setOpen(false);
    } catch {
      // 调用方已经显示可操作的错误提示，保留对话框方便用户调整。
    }
  };
  return <>
    <button className={state.phase === "idle" ? "action-btn-standalone" : "action-btn-standalone active"} onClick={() => setOpen(true)} title="队列完成后的系统操作">
      <Power size={14} />
    </button>
    {open && <div className="modal-layer" onMouseDown={(event) => event.target === event.currentTarget && setOpen(false)}>
      <section className="power-action-dialog" role="dialog" aria-modal="true" aria-labelledby="power-action-title">
        <header><div><h2 id="power-action-title">完成后系统操作</h2><p>一次性设置，重启后不保留。</p></div><button onClick={() => setOpen(false)} aria-label="关闭"><X size={16} /></button></header>
        {state.phase !== "idle" && <div className="power-action-current"><b>当前：{actionText[state.action]}</b><span>{state.message}</span></div>}
        <div className="power-action-options">
          <button onClick={() => void arm("shutdown")}><Power size={18} /><span><b>完成后关机</b><small>任务完成倒计时后关机</small></span></button>
          <button onClick={() => void arm("hibernate")}><Moon size={18} /><span><b>完成后休眠</b><small>保留会话，可快速恢复</small></span></button>
        </div>
        <footer><span>失败、暂停或取消任务将阻止执行。</span>{state.phase !== "idle" && <button className="secondary-button" onClick={() => void onCancel().then(() => setOpen(false))}>取消</button>}</footer>
      </section>
    </div>}
  </>;
}

export function PowerActionBanner({ state, onCancel }: Pick<Props, "state" | "onCancel">) {
  if (state.phase === "idle") return null;
  const title = state.phase === "countdown"
    ? `${state.remaining_seconds} 秒后${actionText[state.action]}`
    : state.phase === "blocked"
      ? `${actionText[state.action]}已暂停`
      : `队列完成后${actionText[state.action]}`;
  return <div className={`power-action-banner ${state.phase}`} role="status">
    <Power size={15} /><div><b>{title}</b><span>{state.message} · 跟踪 {state.target_count} 个任务</span></div>
    <button onClick={() => void onCancel()}>取消</button>
  </div>;
}
