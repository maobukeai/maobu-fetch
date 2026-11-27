import { useLayoutEffect, useRef, useState } from "react";
import type { DesktopCompletionNotice } from "./desktop-completion-api";

export function useCompletionPresentation(
  notice: DesktopCompletionNotice | null | undefined,
  present: (notice: DesktopCompletionNotice, height: number) => Promise<boolean>,
  retryKey: number,
) {
  const root = useRef<HTMLElement>(null);
  const lastSize = useRef("");
  const [presentedId, setPresentedId] = useState<number | undefined>(undefined);
  useLayoutEffect(() => {
    const element = root.current;
    if (!notice || !element) { lastSize.current = ""; return; }
    let disposed = false, presenting = false, desiredHeight = 0, failures = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const show = async () => {
      const height = desiredHeight;
      const key = `${notice.notice_id}:${height}`;
      if (disposed || presenting || failures >= 3 || height <= 0 || key === lastSize.current) return;
      clearTimeout(retry);
      presenting = true;
      const shown = await present(notice, height);
      presenting = false;
      if (disposed) return;
      if (shown) {
        lastSize.current = key;
        setPresentedId(notice.notice_id);
        failures = 0;
        if (desiredHeight !== height) void show();
      } else {
        setPresentedId(undefined);
        failures += 1;
        if (failures < 3) retry = setTimeout(() => { void show(); }, failures * 300);
      }
    };
    const measure = () => {
      desiredHeight = Math.ceil(element.getBoundingClientRect().height);
      void show();
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    // A hidden WebView may suspend animation frames; measure before asking Rust to show it.
    measure();
    return () => { disposed = true; observer.disconnect(); clearTimeout(retry); };
  }, [notice?.notice_id, present, retryKey]);
  return { root, presented: notice != null && presentedId === notice.notice_id };
}
