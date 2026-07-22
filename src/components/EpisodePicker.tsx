import React from "react";
import { CheckSquare, Square, Clock } from "lucide-react";
import { MediaEpisode } from "../types";
import { Select } from "./Select";

const QUALITY_OPTIONS = [
  { value: "best", label: "最高画质 (1080P/4K)" },
  { value: "720p", label: "高清 (720P)" },
  { value: "480p", label: "标清 (480P)" },
];

export interface EpisodePickerProps {
  episodes: MediaEpisode[];
  selectedIndices: Set<number>;
  onChange: (newSelected: Set<number>) => void;
  qualityPreference: string;
  onQualityChange: (quality: string) => void;
}

export function formatDuration(seconds?: number): string {
  if (!seconds || seconds <= 0) return "";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  if (mins >= 60) {
    const hrs = Math.floor(mins / 60);
    const remMins = mins % 60;
    return `${hrs}:${remMins.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}`;
  }
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

export const EpisodePicker: React.FC<EpisodePickerProps> = ({
  episodes,
  selectedIndices,
  onChange,
  qualityPreference,
  onQualityChange,
}) => {
  const allSelected = episodes.length > 0 && selectedIndices.size === episodes.length;

  const handleToggleAll = () => {
    if (allSelected) {
      onChange(new Set());
    } else {
      onChange(new Set(episodes.map((e) => e.index)));
    }
  };

  const handleInvert = () => {
    const next = new Set<number>();
    for (const ep of episodes) {
      if (!selectedIndices.has(ep.index)) {
        next.add(ep.index);
      }
    }
    onChange(next);
  };

  const isDraggingRef = React.useRef(false);
  const dragTargetStateRef = React.useRef<boolean>(true);

  React.useEffect(() => {
    const handleMouseUp = () => {
      isDraggingRef.current = false;
    };
    window.addEventListener("mouseup", handleMouseUp);
    return () => window.removeEventListener("mouseup", handleMouseUp);
  }, []);

  const handleMouseDownItem = (e: React.MouseEvent, index: number) => {
    if (e.button !== 0) return; // 仅限鼠标左键
    isDraggingRef.current = true;
    const willSelect = !selectedIndices.has(index);
    dragTargetStateRef.current = willSelect;
    const next = new Set(selectedIndices);
    if (willSelect) {
      next.add(index);
    } else {
      next.delete(index);
    }
    onChange(next);
  };

  const handleMouseEnterItem = (index: number) => {
    if (!isDraggingRef.current) return;
    const targetState = dragTargetStateRef.current;
    if (selectedIndices.has(index) !== targetState) {
      const next = new Set(selectedIndices);
      if (targetState) {
        next.add(index);
      } else {
        next.delete(index);
      }
      onChange(next);
    }
  };

  return (
    <div className="episode-picker-container" style={{ display: "flex", flexDirection: "column", gap: "8px", marginTop: "4px" }}>
      <div
        className="episode-picker-toolbar"
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          flexWrap: "nowrap",
          whiteSpace: "nowrap",
          gap: "8px",
          padding: "4px 8px",
          background: "var(--card-bg, rgba(255, 255, 255, 0.04))",
          borderRadius: "6px",
          border: "1px solid var(--border-color, rgba(255, 255, 255, 0.08))",
          fontSize: "11.5px",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "6px", flexWrap: "nowrap", flexShrink: 0 }}>
          <span style={{ fontWeight: 600, color: "var(--text-primary)", whiteSpace: "nowrap", fontSize: "11px" }}>
            已选 <span style={{ color: "var(--accent, #0078d4)" }}>{selectedIndices.size}</span>/{episodes.length} 集
          </span>
          <div style={{ display: "flex", gap: "4px" }}>
            <button
              type="button"
              className="input-button compact"
              onClick={handleToggleAll}
              style={{
                padding: "0 6px",
                fontSize: "11px",
                whiteSpace: "nowrap",
                height: "22px",
                minHeight: "22px",
                lineHeight: "22px",
                cursor: "pointer",
              }}
            >
              {allSelected ? "取消全选" : "全选"}
            </button>
            <button
              type="button"
              className="input-button compact"
              onClick={handleInvert}
              style={{
                padding: "0 6px",
                fontSize: "11px",
                whiteSpace: "nowrap",
                height: "22px",
                minHeight: "22px",
                lineHeight: "22px",
                cursor: "pointer",
              }}
            >
              反选
            </button>
          </div>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: "4px", flexWrap: "nowrap", flexShrink: 0 }}>
          <span style={{ color: "var(--text-secondary)", fontSize: "11px", whiteSpace: "nowrap" }}>画质偏好：</span>
          <Select
            value={qualityPreference}
            onChange={(nextQ) => onQualityChange(nextQ)}
            options={QUALITY_OPTIONS}
            style={{ height: "22px", padding: "0 6px", fontSize: "11px" }}
            ariaLabel="画质偏好"
          />
        </div>
      </div>

      <div
        className="episode-picker-list"
        style={{
          maxHeight: "180px",
          overflowY: "auto",
          display: "flex",
          flexDirection: "column",
          gap: "4px",
          paddingRight: "4px",
          userSelect: "none",
          WebkitUserSelect: "none",
        }}
      >
        {episodes.map((ep) => {
          const isSelected = selectedIndices.has(ep.index);
          return (
            <div
              key={ep.index}
              onMouseDown={(e) => handleMouseDownItem(e, ep.index)}
              onMouseEnter={() => handleMouseEnterItem(ep.index)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                padding: "6px 10px",
                borderRadius: "5px",
                background: isSelected ? "var(--accent-bg-subtle, rgba(0, 120, 212, 0.1))" : "var(--item-bg, rgba(255, 255, 255, 0.02))",
                border: isSelected ? "1px solid var(--accent, #0078d4)" : "1px solid var(--border-color, rgba(255, 255, 255, 0.05))",
                cursor: "pointer",
                transition: "all 0.15s ease",
                userSelect: "none",
              }}
            >
              <div style={{ color: isSelected ? "var(--accent, #0078d4)" : "var(--text-tertiary)", display: "flex", alignItems: "center" }}>
                {isSelected ? <CheckSquare size={14} /> : <Square size={14} />}
              </div>
              <span
                style={{
                  padding: "1px 6px",
                  borderRadius: "3px",
                  fontSize: "10px",
                  fontWeight: 600,
                  background: isSelected ? "var(--accent, #0078d4)" : "rgba(255, 255, 255, 0.1)",
                  color: isSelected ? "#fff" : "var(--text-secondary)",
                }}
              >
                P{ep.index}
              </span>
              <span
                style={{
                  flex: 1,
                  fontSize: "12px",
                  color: isSelected ? "var(--text-primary)" : "var(--text-secondary)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
                title={ep.title}
              >
                {ep.title}
              </span>
              {ep.duration && (
                <span style={{ fontSize: "11px", color: "var(--text-tertiary)", display: "flex", alignItems: "center", gap: "3px" }}>
                  <Clock size={11} />
                  {formatDuration(ep.duration)}
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};
