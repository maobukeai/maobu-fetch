import { useEffect, useRef } from "react";

/** Pauses preserve the remaining interval; an unfocused desktop window still expires. */
export function useCompletionCountdown(
  noticeId: number | undefined,
  durationSeconds: number,
  paused: boolean,
  onExpire: () => void,
) {
  const remaining = useRef(0);
  const expire = useRef(onExpire);
  useEffect(() => { expire.current = onExpire; }, [onExpire]);
  useEffect(() => {
    remaining.current = Math.max(0, durationSeconds) * 1000;
  }, [noticeId, durationSeconds]);

  useEffect(() => {
    if (noticeId === undefined || durationSeconds <= 0 || paused) return;
    const started = performance.now();
    const timer = window.setTimeout(() => {
      remaining.current = 0;
      expire.current();
    }, remaining.current);
    return () => {
      window.clearTimeout(timer);
      remaining.current = Math.max(0, remaining.current - (performance.now() - started));
    };
  }, [noticeId, durationSeconds, paused]);
}
