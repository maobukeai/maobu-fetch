import { useCallback, useEffect, useRef } from "react";
import type { FocusEvent } from "react";
import { progressApi, type ProgressWindowSnapshot } from "./progress-api";
import { createRevealQueue } from "./reveal-queue";

/** The bottom strip never moves when the native window grows upwards. */
export function useProgressReveal(
  revealed: boolean,
  accept: (value: ProgressWindowSnapshot) => void,
  failed: () => void,
  active: boolean,
) {
  const root = useRef<HTMLElement>(null);
  const summary = useRef<HTMLButtonElement>(null);
  const queue = useRef<ReturnType<typeof createRevealQueue<ProgressWindowSnapshot>> | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hover = useRef(false);
  const focused = useRef(false);
  const suppressed = useRef(false);
  const initialRevealed = useRef(revealed);

  const clearTimer = useCallback(() => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  }, []);
  const reconcile = useCallback(() => {
    clearTimer();
    if ((hover.current || focused.current) && !suppressed.current) queue.current?.request(true);
    else timer.current = setTimeout(() => queue.current?.request(false), 150);
  }, [clearTimer]);

  useEffect(() => {
    const next = createRevealQueue(progressApi.revealed, accept, failed);
    next.adopt(initialRevealed.current);
    queue.current = next;
    return () => { clearTimer(); next.dispose(); queue.current = null; };
  }, [accept, failed, clearTimer]);
  useEffect(() => { queue.current?.adopt(revealed); }, [revealed]);

  const reset = useCallback(() => {
    clearTimer();
    hover.current = false;
    focused.current = false;
    suppressed.current = false;
    queue.current?.request(false);
  }, [clearTimer]);
  useEffect(() => {
    window.addEventListener("blur", reset);
    return () => window.removeEventListener("blur", reset);
  }, [reset]);
  useEffect(() => { if (!active) reset(); }, [active, reset]);

  const collapse = useCallback(() => {
    clearTimer();
    suppressed.current = true;
    // Keep a keyboard destination mounted when the detail controls are removed.
    summary.current?.focus({ preventScroll: true });
    queue.current?.request(false);
  }, [clearTimer]);
  const reveal = useCallback(() => {
    clearTimer();
    suppressed.current = false;
    queue.current?.request(true);
  }, [clearTimer]);
  const onFocusCapture = useCallback((event: FocusEvent<HTMLElement>) => {
    focused.current = true;
    if (!(event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget))) suppressed.current = false;
    reconcile();
  }, [reconcile]);
  const onBlurCapture = useCallback((event: FocusEvent<HTMLElement>) => {
    focused.current = event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget);
    if (!focused.current && !hover.current) suppressed.current = false;
    reconcile();
  }, [reconcile]);

  return {
    root, summary, collapse, reset, onFocusCapture, onBlurCapture,
    onPointerEnter: () => { hover.current = true; suppressed.current = false; reconcile(); },
    onPointerLeave: () => { hover.current = false; if (!focused.current) suppressed.current = false; reconcile(); },
    reveal,
  };
}
