import { useEffect, useRef } from "react";
import type { ImageItem } from "../../types";
import { formatBytes } from "../../formatters";

interface ViewerThumbnailStripProps {
  gallery: ImageItem[];
  currentIndex: number;
  resolveUrl: (path: string) => string;
  onSelect: (item: ImageItem) => void;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
}

export function ViewerThumbnailStrip({
  gallery,
  currentIndex,
  resolveUrl,
  onSelect,
  onPointerEnter,
  onPointerLeave,
}: ViewerThumbnailStripProps) {
  const activeRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    const el = activeRef.current;
    if (!el) return;
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    el.scrollIntoView({
      behavior: reduced ? "auto" : "smooth",
      inline: "center",
      block: "nearest",
    });
  }, [currentIndex, gallery.length]);

  if (gallery.length === 0) return null;

  return (
    <div
      className="maobu-viewer-thumbnail-strip"
      role="listbox"
      aria-label="缩略图列表"
      onClick={(e) => e.stopPropagation()}
      onMouseEnter={onPointerEnter}
      onMouseLeave={onPointerLeave}
    >
      {gallery.map((item, idx) => {
        const isActive = idx === currentIndex;
        const thumbUrl = resolveUrl(item.path);
        return (
          <button
            key={item.path}
            ref={isActive ? activeRef : undefined}
            type="button"
            role="option"
            aria-selected={isActive}
            onClick={() => onSelect(item)}
            className={`maobu-viewer-thumbnail-item ${isActive ? "active" : ""}`}
            title={`${item.name}${item.size_bytes > 0 ? ` (${formatBytes(item.size_bytes)})` : ""}`}
          >
            <img src={thumbUrl} alt={item.name} loading="lazy" />
          </button>
        );
      })}
    </div>
  );
}
