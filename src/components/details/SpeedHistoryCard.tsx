import { t, useLocale } from "../../i18n";
import { formatBytes } from "../../formatters";
import { computeSpeedSparkGeometry } from "./details-motion";

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
  const width = 280;
  const height = 48;
  const { peak, latest, linePath, areaPath, lastYPercent, isLive } =
    computeSpeedSparkGeometry(samples, width, height);

  return (
    <div className="speed-history-card">
      <div className="speed-history-title">
        <span>{t("details.speedHistoryTitle")}</span>
        <span className="speed-history-readout">
          {latest > 0 && (
            <span className="speed-history-live">
              {formatBytes(latest)}/s
            </span>
          )}
          <span className="speed-history-peak">
            {t("details.speedPeak")} {formatBytes(peak)}/s
          </span>
        </span>
      </div>
      <div className="speed-history-chart-wrap">
        <svg
          className="speed-history-chart"
          viewBox={`0 0 ${width} ${height}`}
          preserveAspectRatio="none"
          role="img"
          aria-label={t("details.speedHistoryTitle")}
        >
          <path d={areaPath} fill="var(--accent, #0078d4)" opacity="0.14" />
          <path
            d={linePath}
            fill="none"
            stroke="var(--accent, #0078d4)"
            strokeWidth="1.9"
            strokeLinecap="round"
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
        {isLive && (
          <span
            className="speed-live-dot"
            style={{ top: `${lastYPercent}%` }}
            aria-hidden="true"
          />
        )}
      </div>
    </div>
  );
}
