import React from "react";
import { FastForward, RotateCcw, SkipForward, X } from "lucide-react";
import { formatDuration } from "../../../formatters";

interface SkipNoticeToastProps {
  type: "intro" | "outro" | null;
  seconds?: number;
  countdown?: number;
  onUndoIntro?: () => void;
  onSkipOutroNow?: () => void;
  onCancelOutroSkip?: () => void;
}

export const SkipNoticeToast: React.FC<SkipNoticeToastProps> = ({
  type,
  seconds = 0,
  countdown = 5,
  onUndoIntro,
  onSkipOutroNow,
  onCancelOutroSkip,
}) => {
  if (!type) return null;

  if (type === "intro") {
    return (
      <div className="maobu-player-skip-toast intro" role="status" aria-live="polite">
        <div className="maobu-player-skip-toast-content">
          <FastForward size={14} className="maobu-player-skip-icon" />
          <span className="maobu-player-skip-text">
            已跳过片头 <strong>{formatDuration(seconds)}</strong>
          </span>
          {onUndoIntro && (
            <button
              type="button"
              onClick={onUndoIntro}
              className="maobu-player-skip-btn undo"
              title="撤销跳过片头，返回开头播放 (按 Z)"
            >
              <RotateCcw size={12} />
              <span>撤销 (Z)</span>
            </button>
          )}
        </div>
      </div>
    );
  }

  if (type === "outro") {
    return (
      <div className="maobu-player-skip-toast outro" role="status" aria-live="polite">
        <div className="maobu-player-skip-toast-content">
          <SkipForward size={14} className="maobu-player-skip-icon" />
          <span className="maobu-player-skip-text">
            即将跳过片尾，播放下一集 ({countdown}s)
          </span>
          <div className="maobu-player-skip-actions">
            {onSkipOutroNow && (
              <button
                type="button"
                onClick={onSkipOutroNow}
                className="maobu-player-skip-btn action-now"
                title="立即播放下一集"
              >
                <span>立即跳过</span>
              </button>
            )}
            {onCancelOutroSkip && (
              <button
                type="button"
                onClick={onCancelOutroSkip}
                className="maobu-player-skip-btn cancel"
                title="取消跳过，继续播放完整片尾"
              >
                <X size={12} />
                <span>取消</span>
              </button>
            )}
          </div>
        </div>
      </div>
    );
  }

  return null;
};
