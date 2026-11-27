import { Copy, FolderOpen, X } from "lucide-react";
import type { ImageFileInfo } from "../../types";
import { formatBytes, formatDate } from "../../formatters";
import { formatAspectRatio } from "./image-viewer-model";

interface ViewerInfoDrawerProps {
  imageInfo: ImageFileInfo | null;
  imageTitle: string;
  naturalSize: { width: number; height: number } | null;
  scale: number;
  onClose: () => void;
  onCopyImage: () => void;
  onCopyPath: () => void;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
}

export function ViewerInfoDrawer({
  imageInfo,
  imageTitle,
  naturalSize,
  scale,
  onClose,
  onCopyImage,
  onCopyPath,
  onPointerEnter,
  onPointerLeave,
}: ViewerInfoDrawerProps) {
  const displayName = imageInfo?.name || imageTitle || "未命名图片";
  const ratioText = naturalSize
    ? formatAspectRatio(naturalSize.width, naturalSize.height)
    : "—";

  return (
    <aside
      className="maobu-viewer-info-modal"
      aria-label="图片属性"
      onClick={(e) => e.stopPropagation()}
      onMouseEnter={onPointerEnter}
      onMouseLeave={onPointerLeave}
    >
      <div className="maobu-viewer-info-title">
        <span>图片属性</span>
        <button
          type="button"
          onClick={onClose}
          title="关闭属性面板"
          className="maobu-viewer-icon-btn maobu-viewer-info-close"
        >
          <X size={12} />
        </button>
      </div>

      <div className="maobu-viewer-info-list">
        <div className="maobu-viewer-info-row">
          <span className="maobu-viewer-info-label">文件名</span>
          <span className="maobu-viewer-info-val" title={displayName}>
            {displayName}
          </span>
        </div>

        {naturalSize && (
          <>
            <div className="maobu-viewer-info-row">
              <span className="maobu-viewer-info-label">分辨率</span>
              <span className="maobu-viewer-info-val">
                {naturalSize.width} × {naturalSize.height}
              </span>
            </div>
            <div className="maobu-viewer-info-row">
              <span className="maobu-viewer-info-label">宽高比</span>
              <span className="maobu-viewer-info-val">{ratioText}</span>
            </div>
          </>
        )}

        <div className="maobu-viewer-info-row">
          <span className="maobu-viewer-info-label">缩放比例</span>
          <span className="maobu-viewer-info-val">{Math.round(scale * 100)}%</span>
        </div>

        {imageInfo && imageInfo.size_bytes > 0 && (
          <div className="maobu-viewer-info-row">
            <span className="maobu-viewer-info-label">文件大小</span>
            <span className="maobu-viewer-info-val">{formatBytes(imageInfo.size_bytes)}</span>
          </div>
        )}

        {imageInfo?.ext && (
          <div className="maobu-viewer-info-row">
            <span className="maobu-viewer-info-label">图像格式</span>
            <span className="maobu-viewer-info-val">{imageInfo.ext.toUpperCase()}</span>
          </div>
        )}

        {imageInfo && imageInfo.modified_ms > 0 && (
          <div className="maobu-viewer-info-row">
            <span className="maobu-viewer-info-label">修改时间</span>
            <span className="maobu-viewer-info-val">{formatDate(imageInfo.modified_ms)}</span>
          </div>
        )}
      </div>

      <div className="maobu-viewer-info-actions">
        <button
          type="button"
          onClick={onCopyImage}
          title="复制图片 (Ctrl+C)"
          className="maobu-viewer-info-action-btn"
        >
          <Copy size={12} />
          <span>复制图片</span>
        </button>
        <button
          type="button"
          onClick={onCopyPath}
          title="复制路径 (Ctrl+Shift+C)"
          className="maobu-viewer-info-action-btn"
        >
          <FolderOpen size={12} />
          <span>复制路径</span>
        </button>
      </div>
    </aside>
  );
}
