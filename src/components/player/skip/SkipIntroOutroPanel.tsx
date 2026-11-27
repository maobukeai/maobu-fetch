import React, { useCallback } from "react";
import {
  Bookmark,
  FastForward,
  RotateCcw,
  Sparkles,
  Tv,
} from "lucide-react";
import { formatPlayerTime } from "../player-ui-model";
import type { ResolvedSkipSettings, SkipIntroOutroConfig } from "./SkipTypes";

interface SkipIntroOutroPanelProps {
  config: SkipIntroOutroConfig;
  resolved: ResolvedSkipSettings;
  currentTime: number;
  duration: number;
  onUpdateConfig: (updater: (prev: SkipIntroOutroConfig) => SkipIntroOutroConfig) => void;
  onApplyToSeries: (introSeconds: number, outroSeconds: number) => void;
  onClearSeriesSetting: () => void;
  onToast: (msg: string) => void;
}

const PRESET_SECONDS = [30, 60, 90, 120];

export const SkipIntroOutroPanel: React.FC<SkipIntroOutroPanelProps> = ({
  config,
  resolved,
  currentTime,
  duration,
  onUpdateConfig,
  onApplyToSeries,
  onClearSeriesSetting,
  onToast,
}) => {
  const adjustIntro = useCallback(
    (delta: number) => {
      onUpdateConfig((prev) => {
        const nextVal = Math.max(
          0,
          Math.min(600, (resolved.introSeconds || prev.defaultIntroSeconds) + delta)
        );
        return { ...prev, defaultIntroSeconds: nextVal };
      });
    },
    [onUpdateConfig, resolved.introSeconds]
  );

  const adjustOutro = useCallback(
    (delta: number) => {
      onUpdateConfig((prev) => {
        const nextVal = Math.max(
          0,
          Math.min(600, (resolved.outroSeconds || prev.defaultOutroSeconds) + delta)
        );
        return { ...prev, defaultOutroSeconds: nextVal };
      });
    },
    [onUpdateConfig, resolved.outroSeconds]
  );

  const markCurrentAsIntro = useCallback(() => {
    if (currentTime <= 0 || (duration > 0 && currentTime > duration * 0.5)) {
      onToast("当前播放位置不适合作为片头（需位于前50%）");
      return;
    }
    const secs = Math.round(currentTime);
    onUpdateConfig((prev) => ({ ...prev, defaultIntroSeconds: secs }));
    onApplyToSeries(secs, resolved.outroSeconds);
    onToast(`已将当前位置 ${formatPlayerTime(secs)} 标记为本剧片头`);
  }, [currentTime, duration, onApplyToSeries, onToast, onUpdateConfig, resolved.outroSeconds]);

  const markCurrentAsOutro = useCallback(() => {
    if (duration <= 0 || currentTime < duration * 0.5) {
      onToast("当前播放位置不适合作为片尾（需位于后50%）");
      return;
    }
    const secs = Math.round(duration - currentTime);
    onUpdateConfig((prev) => ({ ...prev, defaultOutroSeconds: secs }));
    onApplyToSeries(resolved.introSeconds, secs);
    onToast(`已将剩余时长 ${formatPlayerTime(secs)} 标记为本剧片尾`);
  }, [currentTime, duration, onApplyToSeries, onToast, onUpdateConfig, resolved.introSeconds]);

  const hasSeriesCustom = Boolean(config.seriesSettings[resolved.seriesKey]);

  return (
    <div
      className="maobu-player-popup-menu maobu-player-subtitle-panel maobu-player-skip-panel"
      onClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="maobu-player-subtitle-header">
        <div className="maobu-player-sub-title-row">
          <FastForward size={14} className="maobu-player-accent-icon" />
          <span className="maobu-player-sub-heading">跳过片头片尾</span>
        </div>
        <button
          type="button"
          onClick={() => {
            onUpdateConfig((prev) => ({ ...prev, enabled: !prev.enabled }));
            onToast(!config.enabled ? "已开启跳过片头片尾" : "已关闭跳过片头片尾");
          }}
          className={`maobu-player-chip-btn ${config.enabled ? "active" : ""}`}
        >
          {config.enabled ? "开" : "关"}
        </button>
      </div>

      {config.enabled && (
        <div className="maobu-player-skip-status-bar">
          <div className="maobu-player-skip-status-left">
            {resolved.source === "chapter" ? (
              <Sparkles size={12} className="maobu-player-accent-icon" />
            ) : (
              <Tv size={12} className="maobu-player-accent-icon" />
            )}
            <span className="maobu-player-skip-status-text" title={resolved.sourceLabel}>
              {resolved.sourceLabel}
            </span>
          </div>
          {hasSeriesCustom && (
            <button
              type="button"
              onClick={onClearSeriesSetting}
              className="maobu-player-skip-reset-btn"
              title="清除本剧专属记忆"
            >
              <RotateCcw size={10} />
              重置
            </button>
          )}
        </div>
      )}

      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>剧集自动识别</span>
          <button
            type="button"
            disabled={!config.enabled}
            onClick={() =>
              onUpdateConfig((prev) => ({ ...prev, autoDetect: !prev.autoDetect }))
            }
            className={`maobu-player-chip-btn ${config.autoDetect && config.enabled ? "active" : ""}`}
          >
            {config.autoDetect ? "自动" : "手动"}
          </button>
        </div>
      </div>

      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>片头</span>
          <span className="maobu-player-val-mono">
            {formatPlayerTime(resolved.introSeconds)}
          </span>
        </div>
        <div className="maobu-player-skip-presets">
          {PRESET_SECONDS.map((sec) => (
            <button
              key={sec}
              type="button"
              disabled={!config.enabled}
              onClick={() => {
                onUpdateConfig((prev) => ({ ...prev, defaultIntroSeconds: sec }));
                onApplyToSeries(sec, resolved.outroSeconds);
              }}
              className={`maobu-player-skip-preset-chip ${resolved.introSeconds === sec ? "active" : ""}`}
            >
              {sec}s
            </button>
          ))}
          <div className="maobu-player-skip-step-group">
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustIntro(-5)}
              className="maobu-player-offset-btn"
            >
              -5s
            </button>
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustIntro(5)}
              className="maobu-player-offset-btn"
            >
              +5s
            </button>
          </div>
        </div>
        <button
          type="button"
          disabled={!config.enabled || duration <= 0}
          onClick={markCurrentAsIntro}
          className="maobu-player-skip-mark-btn"
        >
          <Bookmark size={11} />
          <span>设为片头 ({formatPlayerTime(Math.round(currentTime))})</span>
        </button>
      </div>

      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>片尾</span>
          <span className="maobu-player-val-mono">
            {formatPlayerTime(resolved.outroSeconds)}
          </span>
        </div>
        <div className="maobu-player-skip-presets">
          {PRESET_SECONDS.map((sec) => (
            <button
              key={sec}
              type="button"
              disabled={!config.enabled}
              onClick={() => {
                onUpdateConfig((prev) => ({ ...prev, defaultOutroSeconds: sec }));
                onApplyToSeries(resolved.introSeconds, sec);
              }}
              className={`maobu-player-skip-preset-chip ${resolved.outroSeconds === sec ? "active" : ""}`}
            >
              {sec}s
            </button>
          ))}
          <div className="maobu-player-skip-step-group">
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustOutro(-5)}
              className="maobu-player-offset-btn"
            >
              -5s
            </button>
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustOutro(5)}
              className="maobu-player-offset-btn"
            >
              +5s
            </button>
          </div>
        </div>
        <button
          type="button"
          disabled={!config.enabled || duration <= 0}
          onClick={markCurrentAsOutro}
          className="maobu-player-skip-mark-btn"
        >
          <Bookmark size={11} />
          <span>
            设为片尾 (余{" "}
            {formatPlayerTime(Math.max(0, Math.round(duration - currentTime)))})
          </span>
        </button>
      </div>

      <div className="maobu-player-subtitle-section maobu-player-subtitle-section-last">
        <div className="maobu-player-subtitle-section-title">
          <span>片尾自动播下一集</span>
          <button
            type="button"
            disabled={!config.enabled}
            onClick={() =>
              onUpdateConfig((prev) => ({
                ...prev,
                autoPlayNextOnOutro: !prev.autoPlayNextOnOutro,
              }))
            }
            className={`maobu-player-chip-btn ${config.autoPlayNextOnOutro && config.enabled ? "active" : ""}`}
          >
            {config.autoPlayNextOnOutro ? "自动连播" : "仅跳过"}
          </button>
        </div>
      </div>
    </div>
  );
};
