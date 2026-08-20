import React, { useMemo, useState } from "react";
import {
  Archive,
  File,
  FileAudio,
  FileImage,
  FileText,
  Film,
  Folder,
  KeyRound,
  Loader2,
  Search,
} from "lucide-react";
import { formatBytes } from "../../formatters";

export interface GenericShareFileItem {
  id: string;
  name: string;
  kind: "drive#file" | "drive#folder" | string;
  size: number;
  path: string;
  extension?: string;
  category?: number | string | null;
  mimeType?: string;
}

export interface GenericShareInfo {
  title: string;
  files: GenericShareFileItem[];
  fileCount?: number;
  folderCount?: number;
  totalSize?: number;
  passCodeRequired?: boolean;
}

export interface CloudSharePickerProps {
  platform: "quark" | "pikpak" | "baidu" | string;
  platformDisplayName: string;
  themeColor?: string;
  shareInfo: GenericShareInfo;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
  onVerifyPassCode?: (passCode: string) => void;
  verifyingPassCode?: boolean;
  passCodeError?: string;
  tipText?: React.ReactNode;
}

const DEFAULT_THEME_COLORS: Record<string, string> = {
  quark: "#d97706",
  pikpak: "#6366f1",
  baidu: "#2563eb",
};

export function getGenericFileIcon(item: GenericShareFileItem) {
  if (item.kind === "drive#folder") {
    return <Folder size={14} style={{ color: "#f59e0b", flexShrink: 0 }} />;
  }

  const ext = (
    item.extension ||
    item.name.split(".").pop() ||
    ""
  ).toLowerCase();
  const mime = (item.mimeType || "").toLowerCase();
  const category = Number(item.category);

  if (
    category === 1 ||
    mime.startsWith("video/") ||
    mime.includes("matroska") ||
    ["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "m4v", "rmvb"].includes(ext)
  ) {
    return <Film size={14} style={{ color: "#3b82f6", flexShrink: 0 }} />;
  }
  if (
    category === 2 ||
    mime.startsWith("audio/") ||
    ["mp3", "flac", "wav", "aac", "ogg", "m4a", "ape"].includes(ext)
  ) {
    return <FileAudio size={14} style={{ color: "#10b981", flexShrink: 0 }} />;
  }
  if (
    category === 3 ||
    mime.startsWith("image/") ||
    ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"].includes(ext)
  ) {
    return <FileImage size={14} style={{ color: "#a855f7", flexShrink: 0 }} />;
  }
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "iso", "7-zip"].includes(ext)) {
    return <Archive size={14} style={{ color: "#f97316", flexShrink: 0 }} />;
  }
  if (["txt", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "md"].includes(ext)) {
    return <FileText size={14} style={{ color: "var(--text-secondary, #888)", flexShrink: 0 }} />;
  }
  return <File size={14} style={{ color: "var(--text-secondary, #888)", flexShrink: 0 }} />;
}

export function CloudSharePicker({
  platform,
  platformDisplayName,
  themeColor,
  shareInfo,
  selectedIds,
  onChange,
  onVerifyPassCode,
  verifyingPassCode,
  passCodeError,
  tipText,
}: CloudSharePickerProps) {
  const [search, setSearch] = useState("");
  const [inputPassCode, setInputPassCode] = useState("");

  const brandColor = themeColor || DEFAULT_THEME_COLORS[platform] || "#3b82f6";

  const onlyFiles = useMemo(
    () =>
      shareInfo.files.filter(
        (item) => item.kind !== "drive#folder" && item.kind !== "folder"
      ),
    [shareInfo.files]
  );

  const filteredFiles = useMemo(() => {
    if (!search.trim()) return onlyFiles;
    const query = search.trim().toLowerCase();
    return onlyFiles.filter(
      (f) =>
        f.name.toLowerCase().includes(query) ||
        f.path.toLowerCase().includes(query)
    );
  }, [onlyFiles, search]);

  const selectedFiles = useMemo(
    () => onlyFiles.filter((f) => selectedIds.has(f.id)),
    [onlyFiles, selectedIds]
  );

  const selectedBytes = useMemo(
    () => selectedFiles.reduce((sum, f) => sum + f.size, 0),
    [selectedFiles]
  );

  const allSelected =
    onlyFiles.length > 0 && selectedIds.size === onlyFiles.length;

  const toggle = (id: string) => {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChange(next);
  };

  const selectAll = () => {
    onChange(new Set(onlyFiles.map((f) => f.id)));
  };

  const deselectAll = () => {
    onChange(new Set());
  };

  const invert = () => {
    const next = new Set<string>();
    for (const f of onlyFiles) {
      if (!selectedIds.has(f.id)) next.add(f.id);
    }
    onChange(next);
  };

  // 1. 若需要提取码/密码保护
  if (shareInfo.passCodeRequired) {
    return (
      <div
        className="cloud-share-passcode-box"
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "12px",
          padding: "16px",
          background: "var(--card-bg, rgba(255, 255, 255, 0.04))",
          borderRadius: "8px",
          border: `1px solid ${brandColor}40`,
          marginTop: "6px",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "8px", fontWeight: 600 }}>
          <KeyRound size={16} style={{ color: brandColor }} />
          <span style={{ color: "var(--text-primary)" }}>
            该 {platformDisplayName} 分享受密码保护，请输入提取码
          </span>
        </div>
        <div style={{ display: "flex", gap: "8px" }}>
          <input
            type="text"
            className="input-text"
            placeholder="输入提取码 / 访问密码..."
            value={inputPassCode}
            onChange={(e) => setInputPassCode(e.target.value.trim())}
            onKeyDown={(e) => {
              if (e.key === "Enter" && inputPassCode.trim() && !verifyingPassCode) {
                onVerifyPassCode?.(inputPassCode.trim());
              }
            }}
            style={{
              flex: 1,
              letterSpacing: platform === "baidu" ? "3px" : "normal",
              fontSize: "13px",
              fontWeight: 600,
            }}
          />
          <button
            type="button"
            className="btn btn-primary"
            disabled={!inputPassCode.trim() || verifyingPassCode}
            onClick={() => onVerifyPassCode?.(inputPassCode.trim())}
            style={{
              background: brandColor,
              borderColor: brandColor,
              color: "#fff",
            }}
          >
            {verifyingPassCode ? (
              <>
                <Loader2 size={13} className="spin" /> 验证中...
              </>
            ) : (
              "确定并解析"
            )}
          </button>
        </div>
        {passCodeError && (
          <div style={{ color: "var(--danger, #ef4444)", fontSize: "12px" }}>
            {passCodeError}
          </div>
        )}
      </div>
    );
  }

  // 2. 主列表视图
  return (
    <div
      className="cloud-share-picker-container"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "8px",
        marginTop: "6px",
      }}
    >
      {/* 顶部工具栏与统计 */}
      <div
        className="cloud-share-picker-toolbar"
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          flexWrap: "nowrap",
          gap: "8px",
          padding: "6px 10px",
          background: "var(--card-bg, rgba(255, 255, 255, 0.04))",
          borderRadius: "6px",
          border: "1px solid var(--border-color, rgba(255, 255, 255, 0.08))",
          fontSize: "12px",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "6px", overflow: "hidden" }}>
          <span
            style={{
              fontWeight: 600,
              color: "var(--text-primary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              maxWidth: "240px",
            }}
            title={shareInfo.title}
          >
            {shareInfo.title}
          </span>
          <span style={{ color: "var(--text-secondary, #888)", fontSize: "11px", whiteSpace: "nowrap" }}>
            ({shareInfo.fileCount ?? onlyFiles.length} 个文件
            {shareInfo.folderCount ? ` · ${shareInfo.folderCount} 个文件夹` : ""})
          </span>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: "10px", flexShrink: 0 }}>
          <span style={{ color: "var(--text-secondary, #888)", fontSize: "11.5px" }}>
            已选 {selectedIds.size} / {onlyFiles.length} 项 · 共 {formatBytes(selectedBytes)}
          </span>
          <div style={{ display: "flex", gap: "6px" }}>
            <button
              type="button"
              className="link-button"
              style={{
                fontSize: "11px",
                color: brandColor,
                background: "transparent",
                border: "none",
                cursor: "pointer",
                padding: "2px 4px",
                fontWeight: 500,
              }}
              onClick={allSelected ? deselectAll : selectAll}
            >
              {allSelected ? "全不选" : "全选"}
            </button>
            <button
              type="button"
              className="link-button"
              style={{
                fontSize: "11px",
                color: brandColor,
                background: "transparent",
                border: "none",
                cursor: "pointer",
                padding: "2px 4px",
                fontWeight: 500,
              }}
              onClick={invert}
            >
              反选
            </button>
          </div>
        </div>
      </div>

      {/* 搜索框（文件较多时显示） */}
      {onlyFiles.length > 5 && (
        <div
          style={{
            position: "relative",
            display: "flex",
            alignItems: "center",
          }}
        >
          <Search
            size={13}
            style={{
              position: "absolute",
              left: "8px",
              color: "var(--text-secondary, #888)",
            }}
          />
          <input
            type="text"
            className="input-text"
            placeholder="搜索文件或路径..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            style={{
              paddingLeft: "26px",
              paddingTop: "3px",
              paddingBottom: "3px",
              fontSize: "11.5px",
              height: "26px",
              width: "100%",
            }}
          />
        </div>
      )}

      {/* 文件列表 */}
      <div
        className="cloud-share-file-list"
        style={{
          maxHeight: "220px",
          overflowY: "auto",
          display: "flex",
          flexDirection: "column",
          gap: "2px",
          padding: "4px",
          background: "var(--card-bg, rgba(255, 255, 255, 0.02))",
          borderRadius: "6px",
          border: "1px solid var(--border-color, rgba(255, 255, 255, 0.06))",
        }}
      >
        {filteredFiles.map((file) => {
          const isChecked = selectedIds.has(file.id);
          return (
            <label
              key={file.id}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                padding: "5px 8px",
                borderRadius: "4px",
                cursor: "pointer",
                userSelect: "none",
                fontSize: "12px",
                background: isChecked ? `${brandColor}1f` : "transparent",
                transition: "background 0.1s ease",
              }}
            >
              <input
                type="checkbox"
                checked={isChecked}
                onChange={() => toggle(file.id)}
                style={{ cursor: "pointer", accentColor: brandColor }}
              />
              {getGenericFileIcon(file)}
              <span
                style={{
                  flex: 1,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  color: isChecked ? "var(--text-primary)" : "var(--text-secondary)",
                  fontWeight: isChecked ? 600 : 400,
                }}
                title={file.path || file.name}
              >
                {file.path || file.name}
              </span>
              <span
                style={{
                  fontSize: "11px",
                  color: "var(--text-secondary, #888)",
                  whiteSpace: "nowrap",
                }}
              >
                {formatBytes(file.size)}
              </span>
            </label>
          );
        })}
        {filteredFiles.length === 0 && (
          <div
            style={{
              padding: "16px",
              textAlign: "center",
              fontSize: "12px",
              color: "var(--text-secondary, #888)",
            }}
          >
            {search ? "没有匹配的文件" : "该分享暂无可用文件"}
          </div>
        )}
      </div>

      {/* 底部提示 */}
      {tipText && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            fontSize: "11px",
            color: "var(--text-secondary, #888)",
            padding: "2px 4px",
          }}
        >
          <span>{tipText}</span>
        </div>
      )}
    </div>
  );
}
