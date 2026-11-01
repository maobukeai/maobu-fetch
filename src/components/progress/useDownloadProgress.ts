import { useCallback, useEffect, useRef, useState } from "react";
import { setLocale, t } from "../../i18n";
import { newestSnapshot } from "./progress-model";
import { progressApi, type ProgressWindowSnapshot } from "./progress-api";

export function useDownloadProgress() {
  const [snapshot, setSnapshot] = useState<ProgressWindowSnapshot | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [connectionAttempt, setConnectionAttempt] = useState(0);
  const busyRef = useRef(false);
  const mounted = useRef(true);
  const accept = useCallback((incoming: ProgressWindowSnapshot) => {
    setSnapshot((current) => newestSnapshot(current, incoming));
  }, []);

  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await progressApi.subscribe((incoming) => { if (!disposed) accept(incoming); });
        if (disposed) { stop(); return; }
        unlisten = stop;
        const initial = await progressApi.snapshot();
        if (!disposed) accept(initial);
      } catch {
        if (!disposed) setError(t("progressWindow.connectionError"));
      }
    })();
    return () => { disposed = true; mounted.current = false; unlisten?.(); };
  }, [accept, connectionAttempt]);

  useEffect(() => {
    if (snapshot) setLocale(snapshot.language);
  }, [snapshot?.language]);

  const perform = useCallback(async (key: string, operation: () => Promise<ProgressWindowSnapshot>, errorKey = "progressWindow.actionError") => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(key);
    setError("");
    try {
      const next = await operation();
      if (mounted.current) accept(next);
    } catch {
      // The small window never echoes arbitrary backend strings or credential-bearing URLs.
      if (mounted.current) setError(t(errorKey));
    } finally {
      busyRef.current = false;
      if (mounted.current) setBusy(null);
    }
  }, [accept]);

  const reconnect = useCallback(() => {
    if (busyRef.current) return;
    setError("");
    setConnectionAttempt((value) => value + 1);
  }, []);

  const revealFailed = useCallback(() => {
    if (mounted.current) setError(t("progressWindow.actionError"));
  }, []);

  return { snapshot, error, busy, perform, reconnect, accept, revealFailed };
}
