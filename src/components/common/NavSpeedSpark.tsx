import { useEffect, useRef, useState } from "react";
import { computeSparkHeights } from "./workspace-telemetry";

const SPARK_SLOTS = 5;
const EMPTY_SAMPLES = Array.from({ length: SPARK_SLOTS }, () => 0);

/**
 * 侧栏底部的迷你实时速率波形柱，严格反映真实全局总速变化。
 */
export function NavSpeedSpark({
  totalSpeed,
  activeCount,
}: {
  totalSpeed: number;
  activeCount: number;
}) {
  const [samples, setSamples] = useState<number[]>(EMPTY_SAMPLES);
  const speedRef = useRef<number>(0);
  speedRef.current =
    Number.isFinite(totalSpeed) && totalSpeed > 0 ? totalSpeed : 0;

  useEffect(() => {
    if (activeCount <= 0 && totalSpeed <= 0) {
      setSamples((prev) => (prev.every((v) => v === 0) ? prev : EMPTY_SAMPLES));
      return;
    }
    const safeSpeed = speedRef.current;
    setSamples((prev) => [...prev.slice(1), safeSpeed]);
  }, [totalSpeed, activeCount]);

  useEffect(() => {
    if (activeCount <= 0) return;
    const timer = window.setInterval(() => {
      setSamples((prev) => [...prev.slice(1), speedRef.current]);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [activeCount]);

  const heights = computeSparkHeights(samples, 3, 11);

  return (
    <span
      className={activeCount > 0 ? "nav-speed-spark active" : "nav-speed-spark"}
      aria-hidden="true"
    >
      {heights.map((height, index) => (
        <i key={index} style={{ height: `${height}px` }} />
      ))}
    </span>
  );
}
