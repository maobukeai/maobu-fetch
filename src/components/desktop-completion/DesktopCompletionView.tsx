import { useEffect, useLayoutEffect, useState } from "react";
import { CheckCircle2, FolderOpen, Play, X } from "lucide-react";
import { t, useLocale } from "../../i18n";
import { useCompletionCountdown } from "./useCompletionCountdown";
import { useCompletionPresentation } from "./useCompletionPresentation";
import { useDesktopCompletion } from "./useDesktopCompletion";
import "./desktop-completion.css";

export function DesktopCompletionView() {
  const locale = useLocale();
  const { snapshot, error, busy, act, present, reconnect, attempt } = useDesktopCompletion();
  const notice = snapshot?.notice;
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const { root, presented } = useCompletionPresentation(notice, present, attempt);
  const durationSeconds = snapshot?.duration_seconds ?? 30;
  const countdownPaused = !presented || hovered || focused || busy || Boolean(error);
  useCompletionCountdown(notice?.notice_id, durationSeconds, countdownPaused, () => {
    if (notice) void act(notice, "dismiss");
  });

  useLayoutEffect(() => {
    document.documentElement.classList.add("desktop-completion-page");
    return () => document.documentElement.classList.remove("desktop-completion-page");
  }, []);
  useEffect(() => {
    const apply = () => {
      const page = document.documentElement;
      const scheme = snapshot?.color_scheme ?? "system";
      page.dataset.theme = scheme === "dark" || (scheme === "system" && matchMedia("(prefers-color-scheme: dark)").matches) ? "dark" : "light";
      page.dataset.accent = snapshot?.accent_color ?? "blue";
      page.dataset.windowStyle = "solid";
    };
    apply();
    const media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [snapshot?.color_scheme, snapshot?.accent_color]);
  useEffect(() => { document.title = `${t("desktopCompletion.title")} · ${t("app.name")}`; }, [locale]);
  useEffect(() => {
    const blur = () => setFocused(false);
    const focus = () => setFocused(Boolean(root.current?.contains(document.activeElement)));
    window.addEventListener("blur", blur);
    window.addEventListener("focus", focus);
    return () => {
      window.removeEventListener("blur", blur);
      window.removeEventListener("focus", focus);
    };
  }, [root]);

  if (!notice) return null;
  return <main ref={root} className="desktop-completion-surface" aria-label={t("desktopCompletion.title")}
    onPointerEnter={() => setHovered(true)} onPointerLeave={() => setHovered(false)}
    onFocusCapture={() => setFocused(true)}
    onBlurCapture={(event) => setFocused(event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget))}
    onKeyDown={(event) => {
      if (event.key === "Escape" && !busy) {
        event.preventDefault();
        void act(notice, "dismiss");
      }
    }}>
    <section
      className={`desktop-completion-card${countdownPaused && durationSeconds > 0 ? " is-countdown-paused" : ""}`}
      data-countdown-paused={String(countdownPaused)}
      role="status"
      aria-live="polite"
      aria-busy={busy}
    >
      <span className="desktop-completion-icon" aria-hidden="true"><CheckCircle2 size={15} /></span>
      <div className="desktop-completion-body">
        <strong className="desktop-completion-title" title={notice.title}>{notice.title}</strong>
        <span className="desktop-completion-location" title={notice.body}>{notice.body}</span>
        <div className="desktop-completion-actions">
          <button type="button" className="desktop-completion-primary" disabled={busy} onClick={() => void act(notice, "file")}>
            <Play size={11} aria-hidden="true" />{t("contextMenu.openFile")}
          </button>
          <button type="button" disabled={busy} onClick={() => void act(notice, "folder")}>
            <FolderOpen size={11} aria-hidden="true" />{t("contextMenu.openFolder")}
          </button>
          <button type="button" disabled={busy} onClick={() => void act(notice, "details")}>{t("toasts.viewDetails")}</button>
        </div>
        {(snapshot?.pending_count ?? 0) > 0 && <span className="desktop-completion-pending">{t("desktopCompletion.pending", { count: snapshot?.pending_count ?? 0 })}</span>}
        {error && <div className="desktop-completion-error" role="alert">
          <span>{t(error === "connection" ? "desktopCompletion.connectionError" : "desktopCompletion.actionError")}</span>
          {error === "connection" && <button type="button" disabled={busy} onClick={reconnect}>{t("desktopCompletion.retry")}</button>}
        </div>}
      </div>
      <button type="button" className="desktop-completion-close" disabled={busy} onClick={() => void act(notice, "dismiss")} aria-label={t("common.close")}>
        <X size={13} aria-hidden="true" />
      </button>
      {durationSeconds > 0 && (
        <div className="desktop-completion-countdown" aria-hidden="true">
          <span
            key={`${notice.notice_id}-${durationSeconds}`}
            className="desktop-completion-countdown-fill"
            style={{
              animationDuration: `${durationSeconds}s`,
              animationPlayState: countdownPaused ? "paused" : "running",
            }}
          />
        </div>
      )}
    </section>
  </main>;
}
