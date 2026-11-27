import { useEffect, useRef, useState } from "react";
import { getDetailsDrawerUnmountDelay } from "./details-motion";

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/**
 * Manage mount + expansion lifecycle for the right Details drawer so CSS width/slide
 * transitions run smoothly on both expand and collapse before unmounting.
 */
export function useDetailsDrawer(open: boolean): {
  mounted: boolean;
  expanded: boolean;
} {
  const [mounted, setMounted] = useState(open);
  const [expanded, setExpanded] = useState(open);
  const unmountTimerRef = useRef<number | null>(null);
  const rafOuterRef = useRef<number | null>(null);
  const rafInnerRef = useRef<number | null>(null);

  const clearScheduled = () => {
    if (unmountTimerRef.current !== null) {
      window.clearTimeout(unmountTimerRef.current);
      unmountTimerRef.current = null;
    }
    if (rafOuterRef.current !== null) {
      window.cancelAnimationFrame(rafOuterRef.current);
      rafOuterRef.current = null;
    }
    if (rafInnerRef.current !== null) {
      window.cancelAnimationFrame(rafInnerRef.current);
      rafInnerRef.current = null;
    }
  };

  useEffect(() => {
    clearScheduled();
    const reduced = prefersReducedMotion();

    if (open) {
      setMounted(true);
      if (reduced) {
        setExpanded(true);
      } else {
        // Double rAF guarantees initial width: 0 frame commits before transitioning to 260px.
        rafOuterRef.current = window.requestAnimationFrame(() => {
          rafOuterRef.current = null;
          rafInnerRef.current = window.requestAnimationFrame(() => {
            rafInnerRef.current = null;
            setExpanded(true);
          });
        });
      }
    } else {
      setExpanded(false);
      const delay = getDetailsDrawerUnmountDelay(reduced);
      if (delay <= 0) {
        setMounted(false);
      } else {
        unmountTimerRef.current = window.setTimeout(() => {
          unmountTimerRef.current = null;
          setMounted(false);
        }, delay);
      }
    }

    return clearScheduled;
  }, [open]);

  return { mounted, expanded };
}
