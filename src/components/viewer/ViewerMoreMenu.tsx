import { Copy, FlipHorizontal, Grid, Pause, Play, Sun } from "lucide-react";
import type { ViewerFilterMode } from "./image-viewer-model";

interface ViewerMoreMenuProps {
  flipH: boolean;
  filterMode: ViewerFilterMode;
  isCheckerboard: boolean;
  isPlayingSlideshow: boolean;
  hasThumbnailStrip: boolean;
  onToggleFlip: () => void;
  onCycleFilter: () => void;
  onToggleCheckerboard: () => void;
  onToggleSlideshow: () => void;
  onCopyImage: () => void;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
}

export function ViewerMoreMenu({
  flipH,
  filterMode,
  isCheckerboard,
  isPlayingSlideshow,
  hasThumbnailStrip,
  onToggleFlip,
  onCycleFilter,
  onToggleCheckerboard,
  onToggleSlideshow,
  onCopyImage,
  onPointerEnter,
  onPointerLeave,
}: ViewerMoreMenuProps) {
  return (
    <div
      className={`maobu-viewer-more-menu ${hasThumbnailStrip ? "with-strip" : ""}`}
      role="toolbar"
      aria-label="更多看图工具"
      onClick={(e) => e.stopPropagation()}
      onMouseEnter={onPointerEnter}
      onMouseLeave={onPointerLeave}
    >
      <button
        type="button"
        onClick={onToggleFlip}
        title="水平翻转 (H)"
        className={`maobu-viewer-icon-btn ${flipH ? "active" : ""}`}
      >
        <FlipHorizontal size={13} />
      </button>

      <button
        type="button"
        onClick={onCycleFilter}
        title="色彩滤镜 (高对比/黑白/反色)"
        className={`maobu-viewer-icon-btn ${filterMode !== "none" ? "active" : ""}`}
      >
        <Sun size={13} />
      </button>

      <button
        type="button"
        onClick={onToggleCheckerboard}
        title="透明棋盘格背景 (B)"
        className={`maobu-viewer-icon-btn ${isCheckerboard ? "active" : ""}`}
      >
        <Grid size={13} />
      </button>

      <button
        type="button"
        onClick={onToggleSlideshow}
        title={isPlayingSlideshow ? "暂停幻灯片" : "幻灯片轮播"}
        className={`maobu-viewer-icon-btn ${isPlayingSlideshow ? "active" : ""}`}
      >
        {isPlayingSlideshow ? <Pause size={13} /> : <Play size={13} />}
      </button>

      <button
        type="button"
        onClick={onCopyImage}
        title="复制图片 (Ctrl+C)"
        className="maobu-viewer-icon-btn"
      >
        <Copy size={13} />
      </button>
    </div>
  );
}
