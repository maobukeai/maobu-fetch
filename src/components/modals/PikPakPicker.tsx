import { useMemo, useState } from "react";
import {
  Archive,
  CheckSquare,
  File,
  FileAudio,
  FileImage,
  FileText,
  Film,
  Folder,
  KeyRound,
  Loader2,
  Search,
  Square,
} from "lucide-react";
import type { PikPakFileItem, PikPakShareInfo } from "../../services/pikpak";
import { formatBytes } from "../../formatters";

function getFileIcon(item: PikPakFileItem) {
  if (item.kind === "drive#folder") {
    return <Folder size={14} className="text-amber-500 shrink-0" />;
  }
  const ext = (item.file_extension || item.name.split(".").pop() || "").toLowerCase();
  const mime = item.mime_type || "";

  if (
    mime.startsWith("video/") ||
    ["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "m4v"].includes(ext)
  ) {
    return <Film size={14} className="text-blue-500 shrink-0" />;
  }
  if (
    mime.startsWith("audio/") ||
    ["mp3", "flac", "wav", "aac", "ogg", "m4a", "ape"].includes(ext)
  ) {
    return <FileAudio size={14} className="text-green-500 shrink-0" />;
  }
  if (
    mime.startsWith("image/") ||
    ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"].includes(ext)
  ) {
    return <FileImage size={14} className="text-purple-500 shrink-0" />;
  }
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "iso"].includes(ext)) {
    return <Archive size={14} className="text-orange-500 shrink-0" />;
  }
  if (["txt", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "md"].includes(ext)) {
    return <FileText size={14} className="text-slate-400 shrink-0" />;
  }
  return <File size={14} className="text-slate-400 shrink-0" />;
}

export function PikPakPicker({
  shareInfo,
  selectedIds,
  onChange,
  onVerifyPassCode,
  verifyingPassCode,
  passCodeError,
}: {
  shareInfo: PikPakShareInfo;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
  onVerifyPassCode?: (passCode: string) => void;
  verifyingPassCode?: boolean;
  passCodeError?: string;
}) {
  const [search, setSearch] = useState("");
  const [inputPassCode, setInputPassCode] = useState("");

  const onlyFiles = useMemo(
    () => shareInfo.files.filter((item) => item.kind === "drive#file"),
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

  // 若需要提取码
  if (shareInfo.passCodeRequired) {
    return (
      <div
        className="pikpak-passcode-box"
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "12px",
          padding: "16px",
          background: "var(--card-bg, rgba(255, 255, 255, 0.04))",
          borderRadius: "8px",
          border: "1px solid var(--border-color, rgba(255, 255, 255, 0.1))",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "8px", fontWeight: 600 }}>
          <KeyRound size={16} className="text-amber-500" />
          <span>该 PikPak 分享受密码保护，请输入提取码</span>
        </div>
        <div style={{ display: "flex", gap: "8px" }}>
          <input
            type="text"
            className="input-text"
            placeholder="输入提取码 / 访问密码..."
            value={inputPassCode}
            onChange={(e) => setInputPassCode(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && inputPassCode.trim() && !verifyingPassCode) {
                onVerifyPassCode?.(inputPassCode.trim());
              }
            }}
            style={{ flex: 1 }}
          />
          <button
            type="button"
            className="btn btn-primary"
            disabled={!inputPassCode.trim() || verifyingPassCode}
            onClick={() => onVerifyPassCode?.(inputPassCode.trim())}
          >
            {verifyingPassCode ? (
              <>
                <Loader2 size={13} className="animate-spin" /> 验证中...
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

  return (
    <div
      className="pikpak-picker-container"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "8px",
        marginTop: "6px",
      }}
    >
      {/* 顶部工具栏与统计 */}
      <div
        className="pikpak-picker-toolbar"
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
        <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
          <span style={{ fontWeight: 600, color: "var(--text-primary)" }}>
            {shareInfo.title}
          </span>
          <span style={{ color: "var(--text-secondary, #888)", fontSize: "11px" }}>
            ({shareInfo.fileCount ?? onlyFiles.length} 个文件
            {shareInfo.folderCount > 0 ? ` · ${shareInfo.folderCount} 个文件夹` : ""})
          </span>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          <span style={{ color: "var(--text-secondary, #888)", fontSize: "11.5px" }}>
            已选 {selectedIds.size} / {onlyFiles.length} 项 · 共 {formatBytes(selectedBytes)}
          </span>
          <div style={{ display: "flex", gap: "6px" }}>
            <button
              type="button"
              className="link-button"
              style={{
                fontSize: "11px",
                color: "var(--accent, #0078d4)",
                background: "transparent",
                border: "none",
                cursor: "pointer",
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
                color: "var(--accent, #0078d4)",
                background: "transparent",
                border: "none",
                cursor: "pointer",
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
        className="pikpak-file-list"
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
                padding: "4px 8px",
                borderRadius: "4px",
                cursor: "pointer",
                userSelect: "none",
                fontSize: "12px",
                background: isChecked
                  ? "var(--accent-subtle, rgba(0, 120, 212, 0.08))"
                  : "transparent",
                transition: "background 0.1s ease",
              }}
            >
              <input
                type="checkbox"
                checked={isChecked}
                onChange={() => toggle(file.id)}
                style={{ cursor: "pointer" }}
              />
              {getFileIcon(file)}
              <span
                style={{
                  flex: 1,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  color: isChecked ? "var(--text-primary)" : "var(--text-secondary)",
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
    </div>
  );
}
