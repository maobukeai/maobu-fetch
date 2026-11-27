import { Check, FolderOpen, Plus, Subtitles } from "lucide-react";
import type { SubtitleItem, SubtitleStyleConfig } from "../../types";
import type { SubtitleCue } from "./SubtitleParser";
import { SUBTITLE_COLOR_PRESETS } from "./player-ui-model";

interface PlayerSubtitlePopoverProps {
  subtitles: SubtitleCue[];
  subtitleName: string;
  subtitleOffset: number;
  matchedSubtitles: SubtitleItem[];
  activeSubtitlePath: string;
  subStyle: SubtitleStyleConfig;
  onImportClick: () => void;
  onClearSubtitle: () => void;
  onLoadSubtitle: (sub: SubtitleItem) => void;
  onUpdateSubStyle: (updater: (prev: SubtitleStyleConfig) => SubtitleStyleConfig) => void;
  onChangeOffset: (nextOffset: number | ((prev: number) => number)) => void;
}

export function PlayerSubtitlePopover({
  subtitles,
  subtitleName,
  subtitleOffset,
  matchedSubtitles,
  activeSubtitlePath,
  subStyle,
  onImportClick,
  onClearSubtitle,
  onLoadSubtitle,
  onUpdateSubStyle,
  onChangeOffset,
}: PlayerSubtitlePopoverProps) {
  return (
    <div
      className="maobu-player-popup-menu maobu-player-subtitle-panel"
      onClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="maobu-player-subtitle-header">
        <div className="maobu-player-sub-title-row">
          <Subtitles size={14} className="maobu-player-accent-icon" />
          <span className="maobu-player-sub-heading">字幕</span>
          {subtitles.length > 0 && (
            <span className="maobu-player-sub-badge">{subtitles.length}</span>
          )}
        </div>
        <div className="maobu-player-sub-header-actions">
          <button
            type="button"
            onClick={onImportClick}
            className="maobu-player-sub-import-btn"
            title="选择本地字幕文件 (.srt / .vtt / .ass / .ssa / .txt)"
          >
            <Plus size={11} />
            <span>导入</span>
          </button>
          {subtitleName && (
            <button
              type="button"
              onClick={onClearSubtitle}
              className="maobu-player-sub-remove-btn"
              title="关闭外挂字幕"
            >
              关闭
            </button>
          )}
        </div>
      </div>

      {matchedSubtitles.length > 0 ? (
        <div className="maobu-player-subtitle-section">
          <div className="maobu-player-subtitle-section-title">
            <span>字幕源 ({matchedSubtitles.length})</span>
          </div>
          <div className="maobu-player-subtitle-list">
            {matchedSubtitles.map((sub) => {
              const isActive =
                activeSubtitlePath === sub.path || subtitleName === sub.name;
              return (
                <button
                  key={sub.path}
                  type="button"
                  onClick={() => onLoadSubtitle(sub)}
                  className={`maobu-player-subtitle-item ${isActive ? "active" : ""}`}
                  title={sub.path}
                >
                  <span className="maobu-player-subtitle-item-ext">
                    {sub.ext.toUpperCase()}
                  </span>
                  <span className="maobu-player-subtitle-item-name">{sub.name}</span>
                  {isActive && (
                    <Check size={12} className="maobu-player-accent-icon shrink-0" />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      ) : (
        <div
          className="maobu-player-subtitle-empty-box"
          onClick={onImportClick}
          title="选择本地字幕文件"
        >
          <FolderOpen size={14} className="maobu-player-accent-icon" />
          <span className="maobu-player-sub-empty-title">导入本地字幕</span>
          <span className="maobu-player-sub-empty-hint">.srt · .vtt · .ass</span>
        </div>
      )}

      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>字号</span>
          <span className="maobu-player-val-mono">{subStyle.fontSize}px</span>
        </div>
        <div className="maobu-player-slider-row">
          <span className="maobu-player-slider-edge">小</span>
          <input
            type="range"
            min="14"
            max="36"
            step="1"
            value={subStyle.fontSize}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10) || 20;
              onUpdateSubStyle((prev) => ({ ...prev, fontSize: v }));
            }}
            className="maobu-player-sub-slider"
          />
          <span className="maobu-player-slider-edge">大</span>
        </div>
      </div>

      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>配色</span>
        </div>
        <div className="maobu-player-color-palette">
          {SUBTITLE_COLOR_PRESETS.map((item) => (
            <button
              key={item.color}
              type="button"
              onClick={() =>
                onUpdateSubStyle((prev) => ({ ...prev, color: item.color }))
              }
              className={`maobu-player-color-dot ${subStyle.color === item.color ? "active" : ""}`}
              style={{ backgroundColor: item.color }}
              title={item.label}
            >
              {subStyle.color === item.color && (
                <span className="maobu-player-color-check" />
              )}
            </button>
          ))}
        </div>
      </div>

      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>底衬</span>
          <button
            type="button"
            onClick={() =>
              onUpdateSubStyle((prev) => ({
                ...prev,
                bgOpacity: prev.bgOpacity > 0 ? 0 : 0.5,
              }))
            }
            className={`maobu-player-chip-btn ${subStyle.bgOpacity > 0 ? "active" : ""}`}
          >
            {subStyle.bgOpacity > 0 ? "开" : "关"}
          </button>
        </div>
      </div>

      <div className="maobu-player-subtitle-section">
        <div className="maobu-player-subtitle-section-title">
          <span>距底</span>
          <span className="maobu-player-val-mono">{subStyle.bottomOffset}%</span>
        </div>
        <input
          type="range"
          min="3"
          max="28"
          step="1"
          value={subStyle.bottomOffset}
          onChange={(e) => {
            const v = parseInt(e.target.value, 10) || 7;
            onUpdateSubStyle((prev) => ({ ...prev, bottomOffset: v }));
          }}
          className="maobu-player-sub-slider"
        />
      </div>

      {subtitles.length > 0 && (
        <div className="maobu-player-subtitle-section maobu-player-subtitle-section-last">
          <div className="maobu-player-subtitle-section-title">
            <span>偏移</span>
            <span
              className={`maobu-player-val-mono ${subtitleOffset === 0 ? "muted" : ""}`}
            >
              {subtitleOffset > 0 ? `+${subtitleOffset}` : subtitleOffset}s
            </span>
          </div>
          <div className="maobu-player-offset-row">
            <button
              type="button"
              onClick={() =>
                onChangeOffset((prev) => parseFloat((prev - 0.5).toFixed(1)))
              }
              className="maobu-player-offset-btn"
              title="字幕提前 0.5s"
            >
              -0.5s
            </button>
            {subtitleOffset !== 0 && (
              <button
                type="button"
                onClick={() => onChangeOffset(0)}
                className="maobu-player-offset-btn"
                title="重置时轴偏移"
              >
                重置
              </button>
            )}
            <button
              type="button"
              onClick={() =>
                onChangeOffset((prev) => parseFloat((prev + 0.5).toFixed(1)))
              }
              className="maobu-player-offset-btn"
              title="字幕延后 0.5s"
            >
              +0.5s
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
