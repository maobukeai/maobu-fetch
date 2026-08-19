import {
  AlertCircle,
  Archive,
  CheckCircle2,
  CirclePause,
  Download,
  File,
  FileAudio,
  FileImage,
  FileText,
  Film,
  Magnet,
  MonitorDown,
} from "lucide-react";
import { t, getLocale } from "./i18n";
import type {
  AdvancedFilter,
  ConnectionState,
  DownloadTask,
  DuplicateType,
  FilterKey,
  FilenameCleanupRule,
  ShortcutKeys,
  Tag,
  TaskStatus,
  WaitReason,
} from "./types";
import { expandSequenceUrls, isBtSourceLine } from "./url-sequence";

export function formatBytes(value: number): string {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

export function formatDuration(seconds: number): string {
  if (seconds < 60) return t("format.seconds", { n: Math.max(1, Math.round(seconds)) });
  if (seconds < 3600) return t("format.minutes", { n: Math.ceil(seconds / 60) });
  return t("format.hours", { h: Math.floor(seconds / 3600), m: Math.ceil((seconds % 3600) / 60) });
}

export function formatDate(value: number): string {
  return new Intl.DateTimeFormat(getLocale(), {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

export function formatScheduleTime(epochMsStr: string): string {
  const ms = Number(epochMsStr);
  if (!ms || !Number.isFinite(ms)) return "—";
  return new Intl.DateTimeFormat(getLocale(), {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(ms));
}

export function waitReasonText(reason: WaitReason): string | null {
  switch (reason.kind) {
    case "not-waiting":
      return null;
    case "queued-behind":
      return t("waitReason.queuedBehind", { count: reason.ahead_count });
    case "waiting-media-tools":
      return t("waitReason.waitingMediaTools");
    case "waiting-user-confirmation":
      return t("waitReason.waitingUserConfirmation");
    case "waiting-scheduled-time":
      return t("waitReason.waitingScheduledTime", { time: formatScheduleTime(reason.scheduled_at) });
    case "waiting-concurrency-limit":
      return t("waitReason.waitingConcurrencyLimit", { count: reason.active_count });
    case "paused":
      return t("waitReason.paused");
    case "paused-by-low-disk":
      return t("status.pausedByLowDiskShort");
    case "paused-by-metered":
      return t("waitReason.pausedByMetered");
    case "interrupted":
      return t("waitReason.interrupted");
    case "remote-changed":
      return t("waitReason.remoteChanged");
    case "unknown":
      return t("waitReason.unknown");
  }
}

export function redactedUrl(value: string): string {
  try {
    const url = new URL(value);
    url.username = "";
    url.password = "";
    url.search = "";
    url.hash = "";
    return url.toString();
  } catch {
    return t("format.invalidUrl");
  }
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function safeDisplayName(value: string): string {
  return value.replace(/[<>:"/\\|?*]/g, "_").slice(0, 120);
}

export function extractDomainForHint(url: string): string | null {
  try {
    const parsed = new URL(url.trim());
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
    const host = parsed.hostname;
    if (!host) return null;
    return host.startsWith("www.") ? host.slice(4) : host;
  } catch {
    return null;
  }
}

export function extractFileNameFromUrl(url: string): string {
  try {
    const trimmed = url.trim();
    if (!trimmed) return "";
    const parsed = new URL(trimmed);
    const pathname = parsed.pathname;
    const lastSegment = pathname.substring(pathname.lastIndexOf("/") + 1);
    if (lastSegment) {
      try {
        const decoded = decodeURIComponent(lastSegment);
        if (decoded.trim()) return decoded.trim();
      } catch (_) {
        if (lastSegment.trim()) return lastSegment.trim();
      }
    }
  } catch (_) {
    try {
      const parts = url.split("/");
      const last = parts[parts.length - 1];
      const cleanLast = last.split("?")[0].split("#")[0];
      if (cleanLast) return decodeURIComponent(cleanLast).trim();
    } catch (_) {}
  }
  return "";
}

export function parseMultilineUrls(input: string): {
  lines: string[];
  skippedCount: number;
  duplicateCount: number;
  sequenceExpanded: number;
  sequenceError?: string;
} {
  const rawLines = input
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const seen = new Set<string>();
  const lines: string[] = [];
  let skippedCount = 0;
  let duplicateCount = 0;
  let sequenceExpanded = 0;
  let sequenceError: string | undefined;
  const urlRegex = /https?:\/\/[^\s<>"']+/i;
  const push = (url: string) => {
    if (seen.has(url)) {
      duplicateCount += 1;
      return;
    }
    seen.add(url);
    lines.push(url);
  };
  for (const line of rawLines) {
    if (isBtSourceLine(line)) {
      push(line);
      continue;
    }
    const match = line.match(urlRegex);
    if (!match) {
      skippedCount += 1;
      continue;
    }
    const expanded = expandSequenceUrls(match[0]);
    if (expanded.error) {
      skippedCount += 1;
      sequenceError ??= expanded.error;
      continue;
    }
    if (expanded.urls.length > 1) sequenceExpanded += expanded.urls.length;
    for (const url of expanded.urls) push(url);
  }
  return { lines, skippedCount, duplicateCount, sequenceExpanded, sequenceError };
}

export function applyFilenameCleanup(
  fileName: string,
  rules: FilenameCleanupRule[]
): string {
  let result = fileName;
  const activeRules = [...rules]
    .filter((r) => r.enabled)
    .sort((a, b) => a.priority - b.priority);

  for (const rule of activeRules) {
    if (!rule.pattern) continue;
    try {
      const regex = new RegExp(rule.pattern, "g");
      result = result.replace(regex, rule.replacement);
    } catch (e) {
      console.warn("Filename cleanup rule regexp error:", rule.pattern, e);
    }
  }
  return result;
}

export function isDownloadableUrlForDialog(url: string): boolean {
  try {
    const trimmed = url.trim();
    return /^https?:\/\/[^\s]+$/i.test(trimmed);
  } catch {
    return false;
  }
}

export const DEFAULT_ADVANCED_FILTER: AdvancedFilter = {
  statuses: [],
  domain: "",
  dateFrom: null,
  dateTo: null,
  sizeMin: null,
  sizeMax: null,
  tagIds: [],
  sources: [],
};

export function isAdvancedFilterEmpty(filter: AdvancedFilter): boolean {
  return (
    filter.statuses.length === 0 &&
    !filter.domain.trim() &&
    filter.dateFrom == null &&
    filter.dateTo == null &&
    filter.sizeMin == null &&
    filter.sizeMax == null &&
    filter.tagIds.length === 0 &&
    filter.sources.length === 0
  );
}

export function matchesAdvancedFilter(
  task: DownloadTask,
  filter: AdvancedFilter,
  taskTagList: Tag[]
): boolean {
  if (filter.statuses.length > 0 && !filter.statuses.includes(task.status)) return false;
  if (filter.domain.trim()) {
    const term = filter.domain.trim().toLowerCase();
    const taskHost = hostOf(task.url).toLowerCase();
    if (!taskHost.includes(term)) return false;
  }
  if (filter.dateFrom !== null && task.created_at < filter.dateFrom) return false;
  if (filter.dateTo !== null && task.created_at > filter.dateTo) return false;
  if (filter.sizeMin !== null && task.total_bytes < filter.sizeMin) return false;
  if (filter.sizeMax !== null && task.total_bytes > filter.sizeMax) return false;
  if (filter.tagIds.length > 0) {
    const taskTagIds = new Set(taskTagList.map((t) => t.id));
    if (!filter.tagIds.every((id) => taskTagIds.has(id))) return false;
  }
  if (filter.sources.length > 0 && !filter.sources.includes(task.source)) return false;
  return true;
}

export function parseShortcutEvent(event: KeyboardEvent): string {
  const parts: string[] = [];
  if (event.ctrlKey || event.metaKey) parts.push("Ctrl");
  if (event.shiftKey) parts.push("Shift");
  if (event.altKey) parts.push("Alt");

  const key = event.key;
  if (["Control", "Shift", "Alt", "Meta"].includes(key)) {
    return parts.join("+");
  }

  let keyName = key;
  if (event.code === "Space" || key === " ") keyName = "Space";
  else if (key.length === 1) keyName = key.toUpperCase();

  parts.push(keyName);
  return parts.join("+");
}

export function matchesShortcut(event: KeyboardEvent, targetStr?: string): boolean {
  if (!targetStr) return false;
  const current = parseShortcutEvent(event);
  return current.toLowerCase() === targetStr.toLowerCase();
}

export const DEFAULT_SHORTCUTS: ShortcutKeys = {
  new_task: "Ctrl+N",
  select_all: "Ctrl+A",
  copy_url: "Ctrl+C",
  open_folder: "Ctrl+O",
  toggle_pause: "Space",
  rename_task: "F2",
  delete_task: "Delete",
  delete_file: "Ctrl+D",
};

export const MIN_PRIORITY = -1000;
export const MAX_PRIORITY = 1000;
export const PRIORITY_STEP = 10;
export const clampPriority = (value: number): number =>
  Math.max(MIN_PRIORITY, Math.min(MAX_PRIORITY, value));

export function minutesToHHMM(minutes: number): string {
  const clamped = Math.min(1439, Math.max(0, Math.floor(minutes)));
  const h = Math.floor(clamped / 60);
  const m = clamped % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

export function hhmmToMinutes(value: string): number {
  const match = value.match(/^(\d{1,2}):(\d{1,2})$/);
  if (!match) return 0;
  const hours = Math.min(23, Math.max(0, Number(match[1])));
  const minutes = Math.min(59, Math.max(0, Number(match[2])));
  return hours * 60 + minutes;
}

export function newTagId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `tag-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export function newQuickViewId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `view-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export function getStatusText(): Record<TaskStatus | "parsing", string> {
  return {
    queued: t("status.queued"),
    downloading: t("status.downloading"),
    parsing: t("status.parsing"),
    paused: t("status.paused"),
    completed: t("status.completed"),
    failed: t("status.failed"),
    cancelled: t("status.cancelled"),
    scheduled: t("status.scheduled"),
    verifying: t("status.verifying"),
    "waiting-network": t("status.waiting-network"),
    "remote-changed": t("status.remote-changed"),
    interrupted: t("status.interrupted"),
    "paused-by-low-disk": t("status.paused-by-low-disk"),
    "paused-by-metered": t("status.paused-by-metered"),
  };
}

export function getDuplicateTypeLabel(): Record<DuplicateType, string> {
  return {
    "same-url": t("duplicateType.same-url"),
    "same-final-url": t("duplicateType.same-final-url"),
    "same-target-path": t("duplicateType.same-target-path"),
    "same-checksum": t("duplicateType.same-checksum"),
  };
}

export function getNav(): Array<[FilterKey, string, typeof Download]> {
  return [
    ["all", t("nav.allTasks"), Download],
    ["downloading", t("nav.downloading"), MonitorDown],
    ["queued", t("nav.queued"), Download],
    ["scheduled", t("nav.scheduled"), Download],
    ["paused", t("nav.paused"), CirclePause],
    ["completed", t("nav.completed"), CheckCircle2],
    ["failed", t("nav.failed"), AlertCircle],
  ];
}

export function getCategories(): Array<[FilterKey, string, typeof Download]> {
  return [
    ["video", t("nav.video"), Film],
    ["audio", t("nav.audio"), FileAudio],
    ["images", t("nav.images"), FileImage],
    ["documents", t("nav.documents"), FileText],
    ["archives", t("nav.archives"), Archive],
    ["apps", t("nav.apps"), File],
    ["bt", t("nav.bt") || "BT / 磁力", Magnet],
  ];
}

export function getConnectionStateLabel(): Record<ConnectionState, string> {
  return {
    connecting: t("connectionState.connecting"),
    downloading: t("connectionState.downloading"),
    retrying: t("connectionState.retrying"),
    completed: t("connectionState.completed"),
    failed: t("connectionState.failed"),
    paused: t("connectionState.paused"),
  };
}

export function isDownloadableUrl(url: string): boolean {
  try {
    const trimmed = url.trim();
    if (!/^https?:\/\/[^\s]+$/i.test(trimmed)) {
      return false;
    }

    const parsed = new URL(trimmed);
    const pathname = parsed.pathname.toLowerCase();

    if (pathname === "/" || pathname === "") {
      const search = parsed.search.toLowerCase();
      if (
        search.includes("download") ||
        search.includes("file=") ||
        search.includes("url=")
      ) {
        return true;
      }
      return false;
    }

    const pageExtensions = [
      ".html",
      ".htm",
      ".shtml",
      ".jsp",
      ".php",
      ".asp",
      ".aspx",
    ];
    if (pageExtensions.some((ext) => pathname.endsWith(ext))) {
      const search = parsed.search.toLowerCase();
      if (
        search.includes("download") ||
        search.includes("file=") ||
        search.includes("url=")
      ) {
        return true;
      }
      return false;
    }

    const downloadExtensions = [
      ".zip",
      ".rar",
      ".7z",
      ".tar",
      ".gz",
      ".bz2",
      ".xz",
      ".pkg",
      ".dmg",
      ".iso",
      ".tgz",
      ".exe",
      ".msi",
      ".apk",
      ".ipa",
      ".deb",
      ".rpm",
      ".mp4",
      ".mkv",
      ".avi",
      ".mov",
      ".wmv",
      ".flv",
      ".webm",
      ".m3u8",
      ".ts",
      ".rmvb",
      ".mp3",
      ".flac",
      ".wav",
      ".aac",
      ".ogg",
      ".m4a",
      ".ape",
      ".pdf",
      ".epub",
      ".docx",
      ".xlsx",
      ".pptx",
      ".torrent",
    ];
    const lastSegment = pathname.split("/").pop() || "";
    if (downloadExtensions.some((ext) => lastSegment.endsWith(ext))) {
      return true;
    }

    const mediaDomains = [
      "youtube.com",
      "youtu.be",
      "bilibili.com",
      "b23.tv",
      "douyin.com",
      "iesdouyin.com",
      "douyinvod.com",
      "vimeo.com",
      "tiktok.com",
      "twitter.com",
      "x.com",
      "weibo.com",
    ];
    const hostname = parsed.hostname.toLowerCase();
    if (
      mediaDomains.some(
        (domain) => hostname === domain || hostname.endsWith("." + domain)
      )
    ) {
      return true;
    }

    const downloadKeywords = [
      "/download",
      "/attachment",
      "/file/",
      "/release/",
      "/update/",
    ];
    if (downloadKeywords.some((keyword) => pathname.includes(keyword))) {
      return true;
    }
    const search = parsed.search.toLowerCase();
    if (
      search.includes("download") ||
      search.includes("file=") ||
      search.includes("url=")
    ) {
      return true;
    }

    return false;
  } catch {
    return false;
  }
}

