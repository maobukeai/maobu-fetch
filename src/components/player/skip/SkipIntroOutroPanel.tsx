import React, { useCallback } from "react";
import {
  Bookmark,
  Check,
  FastForward,
  HelpCircle,
  RotateCcw,
  Sparkles,
  Tv,
} from "lucide-react";
import { formatDuration } from "../../../formatters";
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
  // 调整片头秒数
  const adjustIntro = useCallback(
    (delta: number) => {
      onUpdateConfig((prev) => {
        const nextVal = Math.max(0, Math.min(600, (resolved.introSeconds || prev.defaultIntroSeconds) + delta));
        return { ...prev, defaultIntroSeconds: nextVal };
      });
    },
    [onUpdateConfig, resolved.introSeconds]
  );

  // 调整片尾秒数
  const adjustOutro = useCallback(
    (delta: number) => {
      onUpdateConfig((prev) => {
        const nextVal = Math.max(0, Math.min(600, (resolved.outroSeconds || prev.defaultOutroSeconds) + delta));
        return { ...prev, defaultOutroSeconds: nextVal };
      });
    },
    [onUpdateConfig, resolved.outroSeconds]
  );

  // 标记当前时间为片头结束点
  const markCurrentAsIntro = useCallback(() => {
    if (currentTime <= 0 || (duration > 0 && currentTime > duration * 0.5)) {
      onToast("当前播放位置不适合作为片头（需位于前50%）");
      return;
    }
    const secs = Math.round(currentTime);
    onUpdateConfig((prev) => ({ ...prev, defaultIntroSeconds: secs }));
    onApplyToSeries(secs, resolved.outroSeconds);
    onToast(`已将当前位置 ${formatDuration(secs)} 标记为本剧片头`);
  }, [currentTime, duration, onApplyToSeries, onToast, onUpdateConfig, resolved.outroSeconds]);

  // 标记当前时间为片尾开始点
  const markCurrentAsOutro = useCallback(() => {
    if (duration <= 0 || currentTime < duration * 0.5) {
      onToast("当前播放位置不适合作为片尾（需位于后50%）");
      return;
    }
    const secs = Math.round(duration - currentTime);
    onUpdateConfig((prev) => ({ ...prev, defaultOutroSeconds: secs }));
    onApplyToSeries(resolved.introSeconds, secs);
    onToast(`已将剩余时长 ${formatDuration(secs)} 标记为本剧片尾`);
  }, [currentTime, duration, onApplyToSeries, onToast, onUpdateConfig, resolved.introSeconds]);

  const hasSeriesCustom = Boolean(config.seriesSettings[resolved.seriesKey]);

  return (
    <div
      className="maobu-player-popup-menu maobu-player-subtitle-panel maobu-player-skip-panel"
      onClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
    >
      {/* 头部标题与剧集标识 */}
      <div className="maobu-player-subtitle-header">
        <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
          <FastForward size={14} style={{ color: "#38bdf8" }} />
          <span style={{ fontWeight: 600, color: "#f1f5f9" }}>跳过片头片尾</span>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "4px" }}>
          <button
            type="button"
            onClick={() => {
              onUpdateConfig((prev) => ({ ...prev, enabled: !prev.enabled }));
              onToast(!config.enabled ? "已开启跳过片头片尾" : "已关闭跳过片头片尾");
            }}
            className={`maobu-player-chip-btn ${config.enabled ? "active" : ""}`}
            title="开启或关闭跳过片头片尾"
          >
            {config.enabled ? "已开启" : "已关闭"}
          </button>
        </div>
      </div>

      {/* 智能识别状态提示栏 */}
      {config.enabled && (
        <div className="maobu-player-skip-status-bar">
          <div className="maobu-player-skip-status-left">
            {resolved.source === "chapter" ? (
              <Sparkles size={12} style={{ color: "#38bdf8" }} />
            ) : (
              <Tv size={12} style={{ color: "#38bdf8" }} />
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
              title="清除本剧专属记忆，恢复全局预设"
            >
              <RotateCcw size={10} />
              重置
            </button>
          )}
        </div>
      )}

      {/* 看剧默认自动识别开关 */}
      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <div style={{ display: "flex", alignItems: "center", gap: "4px" }}>
            <span>看剧时默认自动识别</span>
            <span
              title="连续剧或包含章节时，自动识别章节与剧集记忆并自动跳过片头片尾"
              style={{ display: "inline-flex", cursor: "help", color: "#64748b" }}
            >
              <HelpCircle size={11} />
            </span>
          </div>
          <button
            type="button"
            disabled={!config.enabled}
            onClick={() =>
              onUpdateConfig((prev) => ({ ...prev, autoDetect: !prev.autoDetect }))
            }
            className={`maobu-player-chip-btn ${config.autoDetect && config.enabled ? "active" : ""}`}
          >
            {config.autoDetect ? "自动识别" : "仅手动"}
          </button>
        </div>
      </div>

      {/* 片头时长设置卡片 */}
      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>片头跳过时长</span>
          <span style={{ fontFamily: "monospace", color: "#38bdf8" }}>
            {resolved.introSeconds}s ({formatDuration(resolved.introSeconds)})
          </span>
        </div>
        {/* 预设胶囊 */}
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
          <div style={{ marginLeft: "auto", display: "flex", gap: "4px" }}>
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustIntro(-5)}
              className="maobu-player-offset-btn"
              title="减少 5 秒"
            >
              -5s
            </button>
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustIntro(5)}
              className="maobu-player-offset-btn"
              title="增加 5 秒"
            >
              +5s
            </button>
          </div>
        </div>
        {/* 一键标记当前时间 */}
        <button
          type="button"
          disabled={!config.enabled || duration <= 0}
          onClick={markCurrentAsIntro}
          className="maobu-player-skip-mark-btn"
          title="将当前视频播放进度标记为片头跳过点"
        >
          <Bookmark size={11} />
          <span>标记当前播放位置为片头 ({formatDuration(Math.round(currentTime))})</span>
        </button>
      </div>

      {/* 片尾时长设置卡片 */}
      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>片尾跳过时长</span>
          <span style={{ fontFamily: "monospace", color: "#38bdf8" }}>
            {resolved.outroSeconds}s ({formatDuration(resolved.outroSeconds)})
          </span>
        </div>
        {/* 预设胶囊 */}
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
          <div style={{ marginLeft: "auto", display: "flex", gap: "4px" }}>
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustOutro(-5)}
              className="maobu-player-offset-btn"
              title="减少 5 秒"
            >
              -5s
            </button>
            <button
              type="button"
              disabled={!config.enabled}
              onClick={() => adjustOutro(5)}
              className="maobu-player-offset-btn"
              title="增加 5 秒"
            >
              +5s
            </button>
          </div>
        </div>
        {/* 一键标记当前时间 */}
        <button
          type="button"
          disabled={!config.enabled || duration <= 0}
          onClick={markCurrentAsOutro}
          className="maobu-player-skip-mark-btn"
          title="将当前距离结尾的剩余时间标记为片尾跳过点"
        >
          <Bookmark size={11} />
          <span>
            标记剩余时间为片尾 (倒数 {formatDuration(Math.max(0, Math.round(duration - currentTime)))})
          </span>
        </button>
      </div>

      {/* 片尾动作：自动播放下一集 */}
      <div className="maobu-player-subtitle-section" style={{ borderBottom: "none", marginBottom: 0, paddingBottom: 0 }}>
        <div className="maobu-player-subtitle-section-title">
          <span>到达片尾时自动跳转下一集</span>
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
            {config.autoPlayNextOnOutro ? "自动连播" : "播完片尾"}
          </button>
        </div>
      </div>
    </div>
  );
};
