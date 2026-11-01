import { useCallback, useEffect, useLayoutEffect } from "react";
import { t, useLocale } from "../../i18n";
import { progressApi } from "./progress-api";
import { ProgressDetails } from "./ProgressDetails";
import { ProgressSummary } from "./ProgressSummary";
import { useDownloadProgress } from "./useDownloadProgress";
import { useProgressReveal } from "./useProgressReveal";
import { useProgressInteraction } from "./useProgressInteraction";
import { completedCount } from "./progress-model";
import { CompletedSummary } from "./CompletedSummary";
import "./progress-window.css";

export function DownloadProgressView() {
  const locale = useLocale();
  const { snapshot, error, busy, perform, reconnect, accept, revealFailed } = useDownloadProgress();
  const revealed = snapshot?.revealed ?? false;
  const completions = completedCount(snapshot);
  const active = !snapshot || Boolean(snapshot.enabled && !snapshot.loading && (snapshot.total_count > 0 || completions > 0));
  const reveal = useProgressReveal(revealed, accept, revealFailed, active);
  const completionProtocol = snapshot?.completed_count !== undefined || snapshot?.completed_tasks !== undefined;
  const interaction = useProgressInteraction(active && completionProtocol, Boolean(busy), accept, revealFailed);

  useLayoutEffect(() => {
    const page = document.documentElement;
    const alreadyPresent = page.classList.contains("progress-window-page");
    page.classList.add("progress-window-page");
    return () => { if (!alreadyPresent) page.classList.remove("progress-window-page"); };
  }, []);
  useEffect(() => {
    const apply = () => {
      const scheme = snapshot?.color_scheme ?? snapshot?.theme ?? "system";
      document.documentElement.dataset.theme = scheme === "dark" || (scheme === "system" && matchMedia("(prefers-color-scheme: dark)").matches) ? "dark" : "light";
      document.documentElement.dataset.accent = snapshot?.accent_color ?? "blue";
      document.documentElement.dataset.windowStyle = "solid";
    };
    apply();
    const media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [snapshot?.color_scheme, snapshot?.theme, snapshot?.accent_color]);
  useEffect(() => { document.title = `${t("progressWindow.title")} · ${t("app.name")}`; }, [locale]);

  const dismiss = useCallback(() => {
    reveal.reset();
    void perform("dismiss", progressApi.dismiss);
  }, [reveal.reset, perform]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      if (revealed) reveal.collapse();
      else dismiss();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [revealed, reveal.collapse, dismiss]);

  return <main ref={reveal.root} className="download-progress-window" data-revealed={revealed}
    data-completed-only={Boolean(snapshot && snapshot.total_count === 0 && completions > 0)}
    aria-label={t("progressWindow.title")} aria-busy={Boolean(busy)}
    onPointerEnter={() => { reveal.onPointerEnter(); interaction.onPointerEnter(); }}
    onPointerLeave={() => { reveal.onPointerLeave(); interaction.onPointerLeave(); }}
    onFocusCapture={event => { reveal.onFocusCapture(event); interaction.onFocusCapture(); }}
    onBlurCapture={event => { reveal.onBlurCapture(event); interaction.onBlurCapture(event); }}>
    {snapshot && snapshot.total_count === 0 && completions > 0
      ? <CompletedSummary snapshot={snapshot} revealed={revealed} busy={busy} hasError={Boolean(error || snapshot.error)} buttonRef={reveal.summary}
        onReveal={reveal.reveal} onDismiss={dismiss} perform={perform} />
      : <ProgressSummary snapshot={snapshot} revealed={revealed} hasError={Boolean(error || snapshot?.error)}
        buttonRef={reveal.summary} onReveal={reveal.reveal} />}
    {revealed && <ProgressDetails snapshot={snapshot} error={error} busy={busy} perform={perform}
      reconnect={reconnect} onDismiss={dismiss} />}
  </main>;
}
