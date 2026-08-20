import { Archive, Download, File, FileAudio, FileImage, FileText, Film } from "lucide-react";
import { t, useLocale } from "../../i18n";
import type { FilterKey, Tag } from "../../types";

export function CatDownloadMark() {
  return (
    <svg viewBox="0 0 1024 1024" aria-hidden="true">
      <rect x="48" y="48" width="928" height="928" rx="220" fill="#f5f5f7" />
      <path
        d="M302 360 358 230l112 78c28-9 56-14 86-14s58 5 86 14l112-78 56 130v214c0 151-113 254-254 254S302 725 302 574V360Z"
        fill="#1d1d1f"
      />
      <path
        d="M556 392v218m-86-82 86 86 86-86"
        fill="none"
        stroke="#f5f5f7"
        strokeWidth="58"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M445 694h222"
        fill="none"
        stroke="#0a84ff"
        strokeWidth="58"
        strokeLinecap="round"
      />
      <circle cx="428" cy="430" r="19" fill="#f5f5f7" />
      <circle cx="684" cy="430" r="19" fill="#f5f5f7" />
      <path
        d="M755 700c86 15 119-50 76-103"
        fill="none"
        stroke="#1d1d1f"
        strokeWidth="48"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function inferCategory(fileName?: string, explicitCategory?: string): string {
  const ext = String(fileName || "").split(".").pop()?.toLowerCase() || "";
  if (["mp4", "mkv", "mov", "webm", "m3u8", "flv", "avi", "wmv", "ts", "rmvb"].includes(ext)) {
    return "video";
  }
  if (["mp3", "wav", "flac", "aac", "m4a", "ogg", "wma", "opus"].includes(ext)) {
    return "audio";
  }
  if (["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico", "avif"].includes(ext)) {
    return "images";
  }
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"].includes(ext)) {
    return "archives";
  }
  if (["exe", "msi", "dmg", "pkg", "appimage", "apk", "deb", "rpm"].includes(ext)) {
    return "apps";
  }
  if (["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv"].includes(ext)) {
    return "documents";
  }
  return explicitCategory || "other";
}

export function FileIcon({ category, fileName }: { category?: string; fileName?: string }) {
  const cat = inferCategory(fileName, category);
  const Icon =
    cat === "video"
      ? Film
      : cat === "audio"
      ? FileAudio
      : cat === "images"
      ? FileImage
      : cat === "archives"
      ? Archive
      : cat === "apps"
      ? File
      : FileText;
  return (
    <span className={`file-type ${cat}`}>
      <Icon size={16} />
    </span>
  );
}

export function TaskTagChips({ tags, max }: { tags: Tag[]; max: number }) {
  if (tags.length === 0) return null;
  const shown = tags.slice(0, max);
  const overflow = tags.length - shown.length;
  return (
    <div
      className="task-tag-chips"
      aria-label={`标签：${tags.map((t) => t.name).join(", ")}`}
    >
      {shown.map((tag) => (
        <span
          key={tag.id}
          className="task-tag-chip"
          style={{ background: tag.color }}
          title={tag.name}
        >
          {tag.name}
        </span>
      ))}
      {overflow > 0 && <span className="task-tag-chip overflow">+{overflow}</span>}
    </div>
  );
}

export function EmptyState({
  filter,
  view,
  onAdd,
}: {
  filter: FilterKey;
  view: "main" | "history";
  onAdd: () => void;
}) {
  useLocale();
  if (view === "history") {
    return (
      <div className="empty-state">
        <Archive size={36} />
        <h2>{t("empty.noHistoryTasks")}</h2>
        <p>{t("empty.noHistoryTasksDesc")}</p>
      </div>
    );
  }
  return (
    <div className="empty-state">
      <Download size={36} />
      <h2>{filter === "all" ? t("empty.noTasks") : t("empty.noTasksInCategory")}</h2>
      <p>{t("empty.noTasksDesc")}</p>
      <button onClick={onAdd}>{t("nav.newTask")}</button>
    </div>
  );
}
