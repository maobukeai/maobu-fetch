import { useCallback, useEffect, useRef } from "react";
import type { FocusEvent } from "react";
import { progressApi, type ProgressWindowSnapshot } from "./progress-api";
import { createRevealQueue } from "./reveal-queue";

/** One serialized hold intent, independent of reveal animations and task actions. */
export function useProgressInteraction(enabled: boolean, busy: boolean,
  accept: (snapshot: ProgressWindowSnapshot) => void, failed: () => void) {
  const hovered = useRef(false), focused = useRef(false), operating = useRef(busy);
  const capable = useRef(enabled), mounted = useRef(false);
  const queue = useRef<ReturnType<typeof createRevealQueue<ProgressWindowSnapshot>> | null>(null);
  capable.current = enabled;
  operating.current = busy;
  const reconcile = useCallback(() => {
    queue.current?.request(capable.current && (hovered.current || focused.current || operating.current));
  }, []);
  useEffect(() => {
    mounted.current = true;
    if (enabled && !queue.current) queue.current = createRevealQueue(progressApi.interacting,
      value => { if (mounted.current) accept(value); }, () => { if (mounted.current) failed(); }, null);
    reconcile();
    return () => {
      mounted.current = false;
      // Keep the queue alive until a pending true is followed by false. This also
      // serializes StrictMode's cleanup/reconnect instead of racing two invokes.
      queue.current?.request(false);
    };
  }, [enabled, accept, failed, reconcile]);
  useEffect(reconcile, [busy, reconcile]);
  useEffect(() => {
    // Opening a file changes keyboard focus while the cursor can remain on the
    // compact strip. Pointer leave, rather than blur, ends that independent hold.
    const blur = () => { focused.current = false; reconcile(); };
    window.addEventListener("blur", blur);
    return () => window.removeEventListener("blur", blur);
  }, [reconcile]);
  return {
    onPointerEnter: () => { hovered.current = true; reconcile(); },
    onPointerLeave: () => { hovered.current = false; reconcile(); },
    onFocusCapture: () => { focused.current = true; reconcile(); },
    onBlurCapture: (event: FocusEvent<HTMLElement>) => {
      focused.current = event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget);
      reconcile();
    },
  };
}
