import { useCallback, useEffect, useLayoutEffect } from "react";
import { t, useLocale } from "../../i18n";
import { progressApi } from "./progress-api";
import { ProgressDetails } from "./ProgressDetails";
import { ProgressSummary } from "./ProgressSummary";
import { useDownloadProgress } from "./useDownloadProgress";
import { useProgressReveal } from "./useProgressReveal";
import "./progress-window.css";

export function DownloadProgressView() {
  const locale = useLocale();
  const { snapshot, error, busy, perform, reconnect, accept, revealFailed } = useDownloadProgress();
  const revealed = snapshot?.revealed ?? false;
  const active = !snapshot || Boolean(snapshot.enabled && !snapshot.loading && snapshot.total_count > 0);
  const reveal = useProgressReveal(revealed, accept, revealFailed, active);

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
    aria-label={t("progressWindow.title")} aria-busy={Boolean(busy)}
    onPointerEnter={reveal.onPointerEnter} onPointerLeave={reveal.onPointerLeave}
    onFocusCapture={reveal.onFocusCapture} onBlurCapture={reveal.onBlurCapture}>
    <ProgressSummary snapshot={snapshot} revealed={revealed} hasError={Boolean(error || snapshot?.error)}
      buttonRef={reveal.summary} onToggle={revealed ? reveal.collapse : reveal.reveal} />
    {revealed && <ProgressDetails snapshot={snapshot} error={error} busy={busy} perform={perform}
      reconnect={reconnect} onDismiss={dismiss} />}
  </main>;
}
