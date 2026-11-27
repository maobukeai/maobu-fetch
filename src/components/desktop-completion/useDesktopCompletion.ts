import { useCallback, useEffect, useRef, useState } from "react";
import { setLocale } from "../../i18n";
import {
  desktopCompletionApi,
  type DesktopCompletionAction,
  type DesktopCompletionNotice,
  type DesktopCompletionSnapshot,
} from "./desktop-completion-api";

export function useDesktopCompletion() {
  const [snapshot, setSnapshot] = useState<DesktopCompletionSnapshot | null>(null);
  const [error, setError] = useState<"connection" | "action" | null>(null);
  const [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const current = useRef<DesktopCompletionSnapshot | null>(null);
  const mounted = useRef(false), operating = useRef(false);
  const accept = useCallback((incoming: DesktopCompletionSnapshot) => {
    if (current.current && incoming.revision <= current.current.revision) return;
    current.current = incoming;
    setSnapshot(incoming);
  }, []);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await desktopCompletionApi.subscribe((incoming) => {
          if (!disposed) accept(incoming);
        });
        if (disposed) { stop(); return; }
        unlisten = stop;
        const initial = await desktopCompletionApi.snapshot();
        if (!disposed) accept(initial);
      } catch {
        if (!disposed) setError("connection");
      }
    })();
    return () => { disposed = true; mounted.current = false; unlisten?.(); };
  }, [accept, attempt]);
  useEffect(() => { if (snapshot) setLocale(snapshot.language); }, [snapshot?.language]);
  useEffect(() => { setError(null); }, [snapshot?.notice?.notice_id]);

  const act = useCallback(async (notice: DesktopCompletionNotice, action: DesktopCompletionAction) => {
    if (operating.current || current.current?.notice?.notice_id !== notice.notice_id) return;
    operating.current = true;
    setBusy(true);
    setError(null);
    try {
      const next = await desktopCompletionApi.act(notice, action);
      if (mounted.current) accept(next);
    } catch {
      if (mounted.current && current.current?.notice?.notice_id === notice.notice_id) setError("action");
    } finally {
      operating.current = false;
      if (mounted.current) setBusy(false);
    }
  }, [accept]);

  const present = useCallback(async (notice: DesktopCompletionNotice, height: number) => {
    try {
      const next = await desktopCompletionApi.present(notice, height);
      if (mounted.current) accept(next);
      return true;
    } catch {
      if (mounted.current && current.current?.notice?.notice_id === notice.notice_id) setError("connection");
      return false;
    }
  }, [accept]);
  const reconnect = useCallback(() => {
    setError(null);
    setAttempt((value) => value + 1);
  }, []);
  return { snapshot, error, busy, act, present, reconnect, attempt };
}
