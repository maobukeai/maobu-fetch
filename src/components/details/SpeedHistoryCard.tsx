import { t, useLocale } from "../../i18n";
import { formatBytes } from "../../formatters";

export function SpeedHistoryCard({ samples }: { samples: number[] }) {
  useLocale();
  if (samples.length < 2) {
    return (
      <div className="speed-history-card">
        <div className="speed-history-title">{t("details.speedHistoryTitle")}</div>
        <div className="speed-history-empty">{t("details.speedHistoryEmpty")}</div>
      </div>
    );
  }
  const peak = Math.max(...samples, 1);
  const width = 280;
  const height = 48;
  const step = width / (samples.length - 1);
  const points = samples.map((value, index) => {
    const x = index * step;
    const y = height - 2 - (value / peak) * (height - 6);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const line = `M ${points.join(" L ")}`;
  const area = `${line} L ${width},${height} L 0,${height} Z`;
  return (
    <div className="speed-history-card">
      <div className="speed-history-title">
        <span>{t("details.speedHistoryTitle")}</span>
        <span className="speed-history-peak">
          {t("details.speedPeak")} {formatBytes(peak)}/s
        </span>
      </div>
      <svg
        className="speed-history-chart"
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        role="img"
        aria-label={t("details.speedHistoryTitle")}
      >
        <path d={area} fill="var(--accent, #0078d4)" opacity="0.12" />
        <path
          d={line}
          fill="none"
          stroke="var(--accent, #0078d4)"
          strokeWidth="1.5"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    </div>
  );
}
