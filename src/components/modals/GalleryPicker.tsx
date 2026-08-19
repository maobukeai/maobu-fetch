import { useEffect, useMemo, useRef } from "react";
import { CheckSquare, FileImage, Square } from "lucide-react";
import type { MediaFormat } from "../../types";
import { formatBytes } from "../../formatters";

export function GalleryPicker({
  formats,
  thumbnail,
  selectedIds,
  onChange,
}: {
  formats: MediaFormat[];
  thumbnail?: string;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
}) {
  const imageItems = useMemo(
    () => formats.filter((item) => item.image_url),
    [formats]
  );
  const allSelected =
    imageItems.length > 0 && selectedIds.size === imageItems.length;

  const toggle = (id: string) => {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChange(next);
  };
  const selectAll = () =>
    onChange(new Set(imageItems.map((item) => item.id)));
  const invert = () => {
    const next = new Set<string>();
    for (const item of imageItems) {
      if (!selectedIds.has(item.id)) next.add(item.id);
    }
    onChange(next);
  };

  const isDraggingRef = useRef(false);
  const dragTargetStateRef = useRef(true);
  useEffect(() => {
    const handleMouseUp = () => {
      isDraggingRef.current = false;
    };
    window.addEventListener("mouseup", handleMouseUp);
    return () => window.removeEventListener("mouseup", handleMouseUp);
  }, []);

  const handleMouseDownItem = (e: React.MouseEvent, id: string) => {
    if (e.button !== 0) return;
    isDraggingRef.current = true;
    const willSelect = !selectedIds.has(id);
    dragTargetStateRef.current = willSelect;
    toggle(id);
  };

  const handleMouseEnterItem = (id: string) => {
    if (!isDraggingRef.current) return;
    const targetState = dragTargetStateRef.current;
    if (selectedIds.has(id) !== targetState) {
      toggle(id);
    }
  };

  if (imageItems.length === 0) {
    return (
      <div className="media-empty-hint">
        未识别到图片直链。该图集可能需要登录态或受到平台限制，可尝试填写 Cookie 后重新分析。
      </div>
    );
  }

  return (
    <div
      className="episode-picker-container"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "8px",
        marginTop: "4px",
      }}
    >
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
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "6px",
            flexWrap: "nowrap",
            flexShrink: 0,
          }}
        >
          <span
            style={{
              fontWeight: 600,
              color: "var(--text-primary)",
              whiteSpace: "nowrap",
              fontSize: "11px",
            }}
          >
            已选{" "}
            <span style={{ color: "var(--accent, #0078d4)" }}>
              {selectedIds.size}
            </span>
            /{imageItems.length} 张
          </span>
          <div style={{ display: "flex", gap: "4px" }}>
            <button
              type="button"
              className="input-button compact"
              onClick={selectAll}
              disabled={allSelected}
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
              onClick={invert}
              disabled={imageItems.length === 0}
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
        role="group"
        aria-label="图集图片选择"
      >
        {imageItems.map((item, index) => {
          const selected = selectedIds.has(item.id);
          const thumbSrc = item.image_url ?? thumbnail ?? "";
          return (
            <div
              key={item.id}
              onMouseDown={(e) => handleMouseDownItem(e, item.id)}
              onMouseEnter={() => handleMouseEnterItem(item.id)}
              title={item.label || `图片 ${index + 1}`}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                padding: "6px 10px",
                borderRadius: "5px",
                background: selected
                  ? "var(--accent-bg-subtle, rgba(0, 120, 212, 0.1))"
                  : "var(--item-bg, rgba(255, 255, 255, 0.02))",
                border: selected
                  ? "1px solid var(--accent, #0078d4)"
                  : "1px solid var(--border-color, rgba(255, 255, 255, 0.05))",
                cursor: "pointer",
                transition: "all 0.15s ease",
                userSelect: "none",
              }}
            >
              <div
                style={{
                  color: selected
                    ? "var(--accent, #0078d4)"
                    : "var(--text-tertiary)",
                  display: "flex",
                  alignItems: "center",
                }}
              >
                {selected ? <CheckSquare size={14} /> : <Square size={14} />}
              </div>
              <span
                style={{
                  padding: "1px 6px",
                  borderRadius: "3px",
                  fontSize: "10px",
                  fontWeight: 600,
                  background: selected
                    ? "var(--accent, #0078d4)"
                    : "rgba(255, 255, 255, 0.1)",
                  color: selected ? "#fff" : "var(--text-secondary)",
                }}
              >
                #{index + 1}
              </span>
              {thumbSrc ? (
                <img
                  src={thumbSrc}
                  alt={item.label || `图片 ${index + 1}`}
                  loading="lazy"
                  referrerPolicy="no-referrer"
                  style={{
                    width: "32px",
                    height: "32px",
                    objectFit: "cover",
                    borderRadius: "3px",
                    flexShrink: 0,
                    background: "var(--card-bg, rgba(255,255,255,0.04))",
                  }}
                  onError={(e) => {
                    const target = e.currentTarget;
                    target.style.visibility = "hidden";
                  }}
                />
              ) : (
                <div
                  style={{
                    width: "32px",
                    height: "32px",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    borderRadius: "3px",
                    background: "var(--card-bg, rgba(255,255,255,0.04))",
                    color: "var(--text-tertiary)",
                    flexShrink: 0,
                  }}
                >
                  <FileImage size={14} />
                </div>
              )}
              <span
                style={{
                  flex: 1,
                  fontSize: "12px",
                  color: selected
                    ? "var(--text-primary)"
                    : "var(--text-secondary)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {item.label || `图片 ${index + 1}`}
              </span>
              <span
                style={{
                  fontSize: "11px",
                  color: "var(--text-tertiary)",
                  whiteSpace: "nowrap",
                }}
              >
                {item.extension ? `${item.extension.toUpperCase()}` : ""}
                {item.file_size ? ` · ${formatBytes(item.file_size)}` : ""}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
