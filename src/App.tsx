import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { open as pickPath, save as savePath } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import {
  AlertCircle, AlertTriangle, Archive, ArrowLeft, Bookmark, Check, CheckCircle2, CheckSquare, ChevronDown, ChevronUp, ChevronsDown, ChevronsUp, CirclePause, Clock, Copy,
  Download, ExternalLink, File, FileAudio, FileImage, FileText, Film, FolderOpen,
  Gauge, Globe2, HelpCircle, Info, ListFilter, LoaderCircle, MonitorDown, MoreHorizontal, Network,
  PanelRightClose, PanelRightOpen, Pause, Play, Plus, RefreshCw, RotateCcw, Save, Search, Settings, Keyboard,
  ShieldCheck, SlidersHorizontal, Sparkles, Square, Tag as TagIcon, Trash2, Unplug, Video, X, Zap,
} from "lucide-react";
import { api, isDesktop } from "./api";
import { Effect, getCurrentWindow } from "@tauri-apps/api/window";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize } from "@tauri-apps/api/dpi";
import type {
  AdvancedFilter, AppInfo, AppSettings, BackoffStrategy, CategoryRule, CategoryRuleType, CollisionPolicy, ColorScheme, CompletionAction, ConnectionState, DeepLinkReceivedPayload, DownloadPreset, DownloadTask, DuplicateCheckResult, DuplicateMatch, DuplicateType, ExtensionCompatibilityResult, FilenameCleanupRule, FilterKey, MediaCredential, MediaCredentialCheckResult, MediaFormat, MediaPlatform, MediaProbeResult, MeteredNetworkDetectedPayload,
  NewTaskRequest, PairingInfo, PlatformCompatibility, PlatformNamingTemplate, PowerAction, PowerActionState, PrecheckResult, ProxyAuth, QuickView, RestorePreview, RetryPolicy, SegmentStatus, SelfcheckReport, Tag,
  TaskConnectionsEvent, TaskNotificationPayload, TaskStatus, TaskTagsMap, TaskTemplate, TaskTemplateTestResult, ToolComponent, ToolStatus, UpdateCheckResult, UrlHistoryEntry, WaitReason, ShortcutKeys,
} from "./types";
import {
  completionActionKind,
  EMPTY_ADVANCED_FILTER,
  getCopyToData,
  getMoveToData,
  getRunCommandData,
  makeCompletionAction,
  mediaPlatformDisplayName,
  supportLevelColor,
  supportLevelLabel,
  TEMPLATE_VARIABLES,
} from "./types";
import { defaultHistoryDateRange, HistoryDateFilter, matchesHistoryDate } from "./components/HistoryDateFilter";
import { Select, SelectOption } from "./components/Select";
import { PowerActionBanner, PowerActionButton } from "./components/PowerActionControl";
import { PrecheckPanel } from "./components/PrecheckPanel";
import { DiagnosisPanel } from "./components/DiagnosisPanel";
import { YouTubeCredentialsModal } from "./components/YouTubeCredentialsModal";
import { CompletionActionEditor, completionActionLabel } from "./components/CompletionActionEditor";
import { BackupRestoreModal } from "./components/BackupRestoreModal";
import { EpisodePicker } from "./components/EpisodePicker";
import { t, setLocale, useLocale } from "./i18n";
import { reorderTaskIdsWithinPriority, TASK_PRIORITY_PRESETS } from "./priority";

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

/** Task 16: 任务优先级上下限与步长。语义：数字越小越优先。 */
const MIN_PRIORITY = -1000;
const MAX_PRIORITY = 1000;
const PRIORITY_STEP = 10;
/** 将 priority clamp 到合法范围。 */
const clampPriority = (value: number): number => Math.max(MIN_PRIORITY, Math.min(MAX_PRIORITY, value));

const statusText: Record<TaskStatus | "parsing", string> = {
  queued: "等待中", downloading: "下载中", parsing: "解析中", paused: "已暂停", completed: "已完成",
  failed: "失败", cancelled: "已取消", scheduled: "已计划", verifying: "校验中", "waiting-network": "等待网络",
  "remote-changed": "远端已变化", interrupted: "已中断", "paused-by-low-disk": "磁盘空间不足已暂停",
  "paused-by-metered": "计量网络已暂停",
};
/** Task 33: 按当前 locale 返回任务状态文案。在组件渲染时调用以响应语言切换。 */
function getStatusText(): Record<TaskStatus | "parsing", string> {
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
const duplicateTypeLabel: Record<DuplicateType, string> = {
  "same-url": "URL 冲突",
  "same-final-url": "最终地址冲突",
  "same-target-path": "目标文件冲突",
  "same-checksum": "已下载过相同文件",
};
/** Task 33: 按当前 locale 返回重复类型文案。 */
function getDuplicateTypeLabel(): Record<DuplicateType, string> {
  return {
    "same-url": t("duplicateType.same-url"),
    "same-final-url": t("duplicateType.same-final-url"),
    "same-target-path": t("duplicateType.same-target-path"),
    "same-checksum": t("duplicateType.same-checksum"),
  };
}
const nav: Array<[FilterKey, string, typeof Download]> = [
  ["all", "全部任务", Download], ["downloading", "正在下载", MonitorDown],
  ["queued", "等待中", Download], ["scheduled", "计划任务", Download],
  ["paused", "已暂停", CirclePause], ["completed", "已完成", CheckCircle2],
  ["failed", "失败", AlertCircle],
];
/** Task 33: 按当前 locale 返回主导航项。 */
function getNav(): Array<[FilterKey, string, typeof Download]> {
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
const categories: Array<[FilterKey, string, typeof Download]> = [
  ["video", "视频", Film], ["audio", "音频", FileAudio], ["images", "图片", FileImage],
  ["documents", "文档", FileText], ["archives", "压缩包", Archive], ["apps", "应用", File],
];
/** Task 33: 按当前 locale 返回分类导航项。 */
function getCategories(): Array<[FilterKey, string, typeof Download]> {
  return [
    ["video", t("nav.video"), Film],
    ["audio", t("nav.audio"), FileAudio],
    ["images", t("nav.images"), FileImage],
    ["documents", t("nav.documents"), FileText],
    ["archives", t("nav.archives"), Archive],
    ["apps", t("nav.apps"), File],
  ];
}
/** Task 33: 按当前 locale 返回连接状态文案。 */
function getConnectionStateLabel(): Record<ConnectionState, string> {
  return {
    connecting: t("connectionState.connecting"),
    downloading: t("connectionState.downloading"),
    retrying: t("connectionState.retrying"),
    completed: t("connectionState.completed"),
    failed: t("connectionState.failed"),
    paused: t("connectionState.paused"),
  };
}
const defaults: AppSettings = {
  download_dir: "", concurrent_downloads: 3, connections_per_download: 8,
  speed_limit_kbps: 0, start_minimized: false, minimize_to_tray: true,
  close_to_tray: false, notifications: true, auto_start: true, theme: "system",
  accent_color: "blue",
  frosted_glass: false,
  language: "zh-CN", intercept_browser_downloads: true, min_file_size_mb: 1,
  clipboard_monitor: false, proxy_mode: "system", proxy_url: "", proxy_username: "",
  proxy_password: "", user_agent: "MaobuFetch/0.5", default_collision_policy: "rename", default_completion_action: "none",
  max_retries: 3, retry_base_seconds: 2, verify_after_download: false,
  media_tool_auto_update: true,
  yt_dlp_path: "", ffmpeg_path: "", ffprobe_path: "", youtube_po_token: "",
  low_memory_mode: false,
  window_width: 1024,
  window_height: 720,
  auto_scale_ui: false,
  default_retry_policy: { connection_timeout_secs: 60, task_timeout_secs: null, max_retries: 5, backoff: "exponential", initial_backoff_ms: 1000, max_backoff_ms: 60000 },
  row_compact: false,
  detail_default_collapsed: true,
  color_scheme: "system",
  archive_days: 30,
  archive_threshold: 100,
  notify_on_complete: true,
  notify_on_failure: true,
  notify_sound_enabled: true,
  notify_failure_sound_enabled: false,
  pac_script_path: null,
  metered_auto_pause: true,
  user_resumed_after_metered: false,
  shortcut_keys: DEFAULT_SHORTCUTS,
};
const defaultPowerActionState: PowerActionState = { action: "none", phase: "idle", remaining_seconds: 0, target_count: 0 };

/**
 * Task 30.5：使用 Web Audio API 程序生成简短提示音，不引入任何音频资源文件。
 *
 * - `completed`：上升三音 C5 → E5 → G5（523.25 / 659.25 / 783.99 Hz），每音 120ms。
 * - `failed`：下降三音 G5 → E5 → C5，每音 160ms（稍慢以表达负面信号）。
 *
 * 使用单例 AudioContext，避免每次播放都重建。浏览器自动播放策略要求
 * 用户已与页面交互过；首次未交互时静默失败（playNotificationSound 不会抛错）。
 */
let sharedAudioContext: AudioContext | null = null;
const getAudioContext = (): AudioContext | null => {
  if (typeof window === "undefined") return null;
  const AudioContextCtor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioContextCtor) return null;
  if (!sharedAudioContext) {
    try {
      sharedAudioContext = new AudioContextCtor();
    } catch {
      return null;
    }
  }
  return sharedAudioContext;
};

const playTone = (ctx: AudioContext, frequency: number, startAt: number, durationMs: number) => {
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = "sine";
  osc.frequency.value = frequency;
  // 简单 ADSR：起始 0 → 峰值 0.18 → 0，避免咔哒声。
  const durationSec = durationMs / 1000;
  gain.gain.setValueAtTime(0, startAt);
  gain.gain.linearRampToValueAtTime(0.18, startAt + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, startAt + durationSec);
  osc.connect(gain);
  gain.connect(ctx.destination);
  osc.start(startAt);
  osc.stop(startAt + durationSec + 0.02);
};

const playNotificationSound = async (kind: "completed" | "failed"): Promise<void> => {
  const ctx = getAudioContext();
  if (!ctx) return;
  try {
    if (ctx.state === "suspended") {
      await ctx.resume();
    }
  } catch {
    // 用户尚未与页面交互，浏览器禁止播放；静默失败。
    return;
  }
  // C5=523.25, E5=659.25, G5=783.99
  const tones = kind === "completed"
    ? [{ freq: 523.25, dur: 120 }, { freq: 659.25, dur: 120 }, { freq: 783.99, dur: 160 }]
    : [{ freq: 783.99, dur: 160 }, { freq: 659.25, dur: 160 }, { freq: 523.25, dur: 220 }];
  let t = ctx.currentTime;
  for (const tone of tones) {
    playTone(ctx, tone.freq, t, tone.dur);
    t += tone.dur / 1000;
  }
};

/**
 * Task 22.4：根据颜色方案计算是否启用深色主题。
 *
 * - `light` 强制浅色；
 * - `dark` 强制深色；
 * - `system` 跟随 `prefers-color-scheme: dark`。
 *
 * 接受字符串字面量，因此旧的 `theme` 字段与新的 `color_scheme` 字段均可传入。
 */
function usesDarkTheme(colorScheme: ColorScheme | AppSettings["theme"]) {
  return colorScheme === "dark" || (colorScheme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
}

async function applyWindowAppearance(frostedGlass: boolean, dark: boolean) {
  document.documentElement.dataset.windowStyle = frostedGlass ? "frosted" : "solid";
  if (!isDesktop()) return;

  const appWindow = getCurrentWindow();
  if (frostedGlass) {
    await appWindow.setEffects({
      effects: [Effect.Acrylic],
      color: dark ? [24, 24, 27, 72] : [246, 248, 252, 56],
    });
  } else {
    await appWindow.clearEffects();
  }
}

function Titlebar() {
  const [isMaximized, setIsMaximized] = useState(false);
  const appWindow = useMemo(() => isDesktop() ? getCurrentWindow() : null, []);

  useEffect(() => {
    if (!appWindow) return;
    void appWindow.isMaximized().then(setIsMaximized);
    let unlisten: (() => void) | undefined;
    appWindow.onResized(() => {
      void appWindow.isMaximized().then(setIsMaximized);
    }).then(fn => { unlisten = fn; });
    return () => { if (unlisten) unlisten(); };
  }, [appWindow]);

  const handleMinimize = () => { void appWindow?.minimize(); };
  const handleMaximize = () => {
    void appWindow?.toggleMaximize();
  };
  const handleClose = () => { void appWindow?.close(); };

  return (
    <div className="window-titlebar" data-tauri-drag-region>
      <div className="window-titlebar-title" data-tauri-drag-region>猫步下载器 · Maobu Fetch</div>
      <div className="window-controls">
        <button className="window-control-btn min" onClick={handleMinimize} title="最小化">
          <svg width="10" height="1" viewBox="0 0 10 1"><rect width="10" height="1" fill="currentColor"/></svg>
        </button>
        <button className="window-control-btn max" onClick={handleMaximize} title={isMaximized ? "向下还原" : "最大化"}>
          {isMaximized ? (
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M1.5,3.5 L1.5,8.5 L6.5,8.5 L6.5,3.5 Z" fill="none" stroke="currentColor" strokeWidth="1" />
              <path d="M3.5,1.5 L8.5,1.5 L8.5,6.5" fill="none" stroke="currentColor" strokeWidth="1" />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10"><rect width="10" height="10" fill="none" stroke="currentColor" strokeWidth="1"/></svg>
          )}
        </button>
        <button className="window-control-btn close" onClick={handleClose} title="关闭">
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M0,0 L10,10 M10,0 L0,10" stroke="currentColor" strokeWidth="1" />
          </svg>
        </button>
      </div>
    </div>
  );
}

function WindowResizeHandles() {
  if (!isDesktop()) return null;

  const handleMouseDown = (direction: string, event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    try {
      const appWindow = getCurrentWindow();
      void appWindow.startResizeDragging(direction as any);
    } catch (err) {
      console.error("Failed to start resize dragging:", err);
    }
  };

  const directions = [
    { key: "top", dir: "North" },
    { key: "bottom", dir: "South" },
    { key: "left", dir: "West" },
    { key: "right", dir: "East" },
    { key: "top-left", dir: "NorthWest" },
    { key: "top-right", dir: "NorthEast" },
    { key: "bottom-left", dir: "SouthWest" },
    { key: "bottom-right", dir: "SouthEast" }
  ];

  return (
    <>
      {directions.map(({ key, dir }) => (
        <div
          key={key}
          className={`resize-handle ${key}`}
          onMouseDown={(e) => handleMouseDown(dir, e)}
        />
      ))}
    </>
  );
}

export default function App() {
  const appWindow = useMemo(() => isDesktop() ? getCurrentWindow() : null, []);
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  // 始终持有最新的 tasks 快照，供 dragRef 闭包读取
  const tasksRef = useRef<DownloadTask[]>([]);
  useEffect(() => { tasksRef.current = tasks; }, [tasks]);
  const [settings, setSettings] = useState(defaults);
  const [loading, setLoading] = useState(true);
  const [fatal, setFatal] = useState<string>();
  const [filter, setFilter] = useState<FilterKey>("all");
  const [search, setSearch] = useState("");
  // Task 33: 订阅 locale 变化，语言切换时所有调用 t() 的组件自动重渲染。
  useLocale();
  // Task 33: 设置变化时同步语言到 i18n 模块。settings.language 由后端 AppSettings 提供，
  // 默认 "zh-CN"；用户在设置页切换语言并保存后，i18n 模块立即生效。
  useEffect(() => {
    if (settings.language) setLocale(settings.language);
  }, [settings.language]);
  const [historyDate, setHistoryDate] = useState(defaultHistoryDateRange);
  // Task 24: 主列表 / 历史视图切换。history 默认按"已完成"筛选。
  const [view, setView] = useState<"main" | "history">("main");
  const [historyStatusFilter, setHistoryStatusFilter] = useState<TaskStatus | "all">("all");
  const [powerAction, setPowerAction] = useState(defaultPowerActionState);
  const [sort, setSort] = useState<{ key: keyof DownloadTask; desc: boolean }>({ key: "queue_position", desc: false });
  const [selected, setSelected] = useState(new Set<string>());
  const [primaryTaskId, setPrimaryTaskId] = useState<string | undefined>(undefined);
  const [showDetails, setShowDetails] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [categoriesExpanded, setCategoriesExpanded] = useState(true);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const [splash, setSplash] = useState(true);
  const [initialUrlFromClipboard, setInitialUrlFromClipboard] = useState("");
  const [toast, setToast] = useState<{ kind: "ok" | "error"; text: string }>();
  const [context, setContext] = useState<{ x: number; y: number; id: string }>();
  const [aboutOpen, setAboutOpen] = useState(false);
  const [columnWidths, setColumnWidths] = useState<Record<string, number>>({});
  const [selfcheckToast, setSelfcheckToast] = useState<{ interrupted: number; dropped: number; taskIds: string[] } | undefined>();
  // Task 16.3: 拖拽排序——使用 mouse 事件替代 HTML5 drag API（WebView2 兼容）。
  // dragRef 跟踪拖拽过程状态；高亮直接操作 DOM class，不使用 setState，防止重渲染影响 elementFromPoint。
  const dragRef = useRef<{ taskId: string; startY: number; active: boolean; hoverId: string | null; sourceEl: HTMLElement; dropEl: HTMLElement | null } | null>(null);
  // notifyRef / refreshRef 始终指向最新的函数，避免 useCallback 闭包过期
  const notifyRef = useRef<(text: string, kind?: "ok" | "error") => void>(() => {});
  const refreshRef = useRef<() => Promise<void>>(() => Promise.resolve());
  // Task 21.1: 快捷键触发的对话框状态。
  // - renameTarget: F2 触发 of 重命名目标任务。
  const [renameTarget, setRenameTarget] = useState<DownloadTask | null>(null);
  const [speedLimitTarget, setSpeedLimitTarget] = useState<DownloadTask | null>(null);
  // Task 30：失败通知 toast（带"一键重试"按钮）。仅在收到 failed kind 的 task-notification 事件时显示。
  const [failureToast, setFailureToast] = useState<{ taskId: string; title: string; body: string } | undefined>();
  const [youtubeModalTaskId, setYoutubeModalTaskId] = useState<string | null>(null);
  // Task 25: 标签 + 任务-标签关联 + 高级筛选 + 快捷视图。
  // - tags: 全部标签列表（按 name 升序）
  // - taskTags: task_id -> Tag[] 映射，用于任务行 chip 和详情页编辑
  // - advancedFilter: 当前高级筛选条件；空条件表示不限制
  // - advancedFilterOpen: 控制高级筛选面板展开/收起
  // - quickViews: localStorage 持久化的快捷视图列表
  const [tags, setTags] = useState<Tag[]>([]);
  const [taskTags, setTaskTags] = useState<TaskTagsMap>({});
  const [advancedFilter, setAdvancedFilter] = useState<AdvancedFilter>(EMPTY_ADVANCED_FILTER);
  const [advancedFilterOpen, setAdvancedFilterOpen] = useState(false);
  const [quickViews, setQuickViews] = useState<QuickView[]>([]);

  // Drag-to-select checkbox batch selection implementation
  const isDraggingSelection = useRef(false);
  const targetCheckedState = useRef(true);

  useEffect(() => {
    const resetDrag = () => {
      isDraggingSelection.current = false;
    };
    window.addEventListener("mouseup", resetDrag);
    window.addEventListener("blur", resetDrag);
    document.addEventListener("mouseleave", resetDrag);
    return () => {
      window.removeEventListener("mouseup", resetDrag);
      window.removeEventListener("blur", resetDrag);
      document.removeEventListener("mouseleave", resetDrag);
    };
  }, []);

  const handleCheckboxMouseDown = (taskId: string, isChecked: boolean, event: React.MouseEvent) => {
    if (event.button !== 0) return;
    isDraggingSelection.current = true;
    targetCheckedState.current = !isChecked;
    setPrimaryTaskId(taskId);
    setSelected((current) => {
      const next = new Set(current);
      if (targetCheckedState.current) {
        next.add(taskId);
      } else {
        next.delete(taskId);
      }
      return next;
    });
  };

  const handleCheckboxMouseEnter = (taskId: string) => {
    if (!isDraggingSelection.current) return;
    setSelected((current) => {
      const next = new Set(current);
      if (targetCheckedState.current) {
        next.add(taskId);
      } else {
        next.delete(taskId);
      }
      return next;
    });
  };

  const refresh = async () => {
    try {
      setTasks(await api.list());
      if (isDesktop()) {
        const [nextSettings, nextPowerAction] = await Promise.all([api.settings(), api.powerActionState()]);
        setSettings(nextSettings);
        setPowerAction(nextPowerAction);
      }
      setFatal(undefined);
    } catch (error) { setFatal(String(error)); }
    finally { setLoading(false); }
  };
  // Task 25: 刷新标签与任务-标签关联。每次 refresh 后调用一次，
  // 保证任务行 chip、详情页编辑器、高级筛选标签下拉都用最新数据。
  const refreshTags = async () => {
    if (!isDesktop()) return;
    try {
      const [nextTags, nextTaskTags] = await Promise.all([api.tagList(), api.taskTagsListAll()]);
      setTags(nextTags);
      setTaskTags(nextTaskTags);
    } catch (error) { /* 标签加载失败不阻塞主流程，仅记录到 toast */ }
  };
  // Task 25: 快捷视图持久化到 localStorage。键名固定为 `maobu.quickViews`，
  // 与 SQLite 设置无关，仅作为前端个人偏好。
  const QUICK_VIEWS_STORAGE_KEY = "maobu.quickViews";
  useEffect(() => {
    try {
      const raw = localStorage.getItem(QUICK_VIEWS_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as QuickView[];
        if (Array.isArray(parsed)) setQuickViews(parsed);
      }
    } catch { /* 旧数据格式损坏时静默忽略，使用空列表 */ }
  }, []);
  useEffect(() => {
    try { localStorage.setItem(QUICK_VIEWS_STORAGE_KEY, JSON.stringify(quickViews)); } catch { /* localStorage 配额或不可用时静默忽略 */ }
  }, [quickViews]);
  useEffect(() => {
    const handleContextMenu = (e: globalThis.MouseEvent) => e.preventDefault();
    document.addEventListener("contextmenu", handleContextMenu);

    const startTime = Date.now();
    void refresh().then(() => {
      void refreshTags();
      const elapsed = Date.now() - startTime;
      const delay = Math.max(0, 800 - elapsed);
      setTimeout(() => {
        const element = document.getElementById("splash-screen");
        if (element) {
          element.classList.add("fade-out");
          setTimeout(() => {
            setSplash(false);
          }, 300);
        } else {
          setSplash(false);
        }
      }, delay);
    });

    let unlisten: Array<() => void> = [];
    void api.subscribe((event) => {
      if ("removed" in event) {
        setTasks((items) => items.filter((task) => task.id !== event.removed));
        setSelected((current) => {
          if (current.has(event.removed)) {
            const next = new Set(current);
            next.delete(event.removed);
            return next;
          }
          return current;
        });
      } else {
        setTasks((items) => items.some((task) => task.id === event.task.id)
          ? items.map((task) => task.id === event.task.id ? event.task : task)
          : [event.task, ...items]);
      }
    }).then((items) => { unlisten.push(...items); });
    void api.subscribeSettings(setSettings).then((item) => {
      if (item) unlisten.push(item);
    });
    void api.subscribePowerAction(setPowerAction).then((item) => {
      if (item) unlisten.push(item);
    });
    void api.subscribeNotificationErrors((message) => setToast({ kind: "error", text: message })).then((item) => {
      if (item) unlisten.push(item);
    });
    void api.subscribeStartupSelfcheck((report: SelfcheckReport) => {
      if (report.interrupted_count > 0 || report.dropped_shards > 0) {
        setSelfcheckToast({
          interrupted: report.interrupted_count,
          dropped: report.dropped_shards,
          taskIds: report.recovered_tasks ?? [],
        });
      }
    }).then((item) => {
      if (item) unlisten.push(item);
    });
    // Task 29：监听 maobu:// 深链错误与 .maobu-task 文件导入事件。
    void api.subscribeDeepLinkErrors((message) => {
      setToast({ kind: "error", text: message });
    }).then((item) => {
      if (item) unlisten.push(item);
    });
    void api.subscribeDeepLinkReceived((payload: DeepLinkReceivedPayload) => {
      if (payload.action === "add" && payload.url) {
        setInitialUrlFromClipboard(payload.url);
        setNewOpen(true);
        if (appWindow) {
          void appWindow.show();
          void appWindow.unminimize();
          void appWindow.setFocus();
        }
      } else if (payload.action === "import") {
        const count = payload.count ?? 0;
        if (count > 0) {
          setToast({ kind: "ok", text: `已导入 ${count} 个任务` });
        }
      }
    }).then((item) => {
      if (item) unlisten.push(item);
    });
    // Task 32.3：监听计量网络自动暂停事件，展示 toast 提示用户。
    // 后端在 60s 定时检查中检测到计量网络且实际暂停了 ≥1 个任务时 emit。
    void api.subscribeMeteredNetwork((payload: MeteredNetworkDetectedPayload) => {
      const count = payload.paused_count ?? 0;
      if (count > 0) {
        setToast({ kind: "error", text: `当前为计量网络，已暂停 ${count} 个任务` });
      }
    }).then((item) => {
      if (item) unlisten.push(item);
    });
    return () => {
      document.removeEventListener("contextmenu", handleContextMenu);
      unlisten.forEach((item) => item());
    };
  }, []);
  // Task 30.2 / 30.5：监听任务完成/失败通知事件，按设置播放提示音并展示失败重试 toast。
  // - 完成事件：根据 settings.notify_sound_enabled 决定是否播放上升提示音。
  // - 失败事件：根据 settings.notify_failure_sound_enabled 决定是否播放下降提示音；
  //   同时弹出带"一键重试"按钮的 toast，用户点击可重试对应任务。
  // 依赖 settings 以读取最新的提示音开关；事件流本身不会重复触发本 effect。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void api.subscribeTaskNotification((payload: TaskNotificationPayload) => {
      if (payload.kind === "completed") {
        if (settings.notify_sound_enabled) {
          void playNotificationSound("completed");
        }
      } else if (payload.kind === "failed") {
        if (settings.notify_failure_sound_enabled) {
          void playNotificationSound("failed");
        }
        setFailureToast({ taskId: payload.task_id, title: payload.title, body: payload.body });
      }
    }).then((item) => {
      if (item) unlisten = item;
    });
    return () => {
      if (unlisten) unlisten();
    };
  }, [settings.notify_sound_enabled, settings.notify_failure_sound_enabled]);
  // Task 30.4：失败重试 toast 自动消失（与普通 toast 不同的 8 秒时长，给用户足够时间点击重试）。
  useEffect(() => {
    if (!failureToast) return;
    const timer = setTimeout(() => setFailureToast(undefined), 8000);
    return () => clearTimeout(timer);
  }, [failureToast]);
  useEffect(() => {
    const applyColorScheme = () => {
      const dark = usesDarkTheme(settings.color_scheme);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
      document.documentElement.dataset.accent = settings.accent_color;
      // Task 22.4：通过 body.classList 同步 light/dark 类，作为 data-theme 的补充标识。
      document.body.classList.toggle("dark", dark);
      document.body.classList.toggle("light", !dark);
      void applyWindowAppearance(settings.frosted_glass, dark).catch((error) => {
        document.documentElement.dataset.windowStyle = "solid";
        setToast({ kind: "error", text: `无法应用磨砂玻璃效果：${String(error)}` });
      });
    };
    applyColorScheme();
    // Task 22.4：仅在 System 模式下监听 prefers-color-scheme 变化；
    // Light/Dark 为强制覆盖，无需监听。
    if (settings.color_scheme !== "system") return;
    const media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", applyColorScheme);
    return () => media.removeEventListener("change", applyColorScheme);
  }, [settings.color_scheme, settings.accent_color, settings.frosted_glass]);
  // Task 22.2：根据 row_compact 切换 body.row-compact 类，控制行高变量。
  useEffect(() => {
    document.body.classList.toggle("row-compact", settings.row_compact);
  }, [settings.row_compact]);
  useEffect(() => {
    if (!appWindow || !settings.window_width || !settings.window_height) return;
    void appWindow.setSize(new LogicalSize(settings.window_width, settings.window_height));
  }, [appWindow, settings.window_width, settings.window_height]);
  useEffect(() => {
    const applyScale = () => {
      if (settings.auto_scale_ui) {
        const baseWidth = 1024;
        const scale = window.outerWidth / baseWidth;
        const clampedScale = Math.min(Math.max(scale, 0.75), 2.0);
        document.documentElement.style.zoom = String(clampedScale);
      } else {
        document.documentElement.style.zoom = "";
      }
    };
    applyScale();
    window.addEventListener("resize", applyScale);
    return () => {
      window.removeEventListener("resize", applyScale);
    };
  }, [settings.auto_scale_ui]);
  useEffect(() => {
    const close = () => setContext(undefined);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, []);
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(undefined), 3500);
    return () => clearTimeout(timer);
  }, [toast]);
  useEffect(() => {
    if (!selfcheckToast) return;
    const timer = setTimeout(() => setSelfcheckToast(undefined), 12000);
    return () => clearTimeout(timer);
  }, [selfcheckToast]);
  const allowClose = useRef(false);

  useEffect(() => {
    if (!appWindow) return;
    const unlistenPromise = appWindow.onCloseRequested(async (event) => {
      if (allowClose.current) {
        return;
      }
      event.preventDefault();
      const rememberAction = localStorage.getItem("remember_close_action");
      if (rememberAction === "tray") {
        await appWindow.hide();
      } else if (rememberAction === "exit") {
        await invoke("app_exit");
      } else {
        setShowCloseConfirm(true);
      }
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [appWindow]);

  const handleCloseConfirm = async (action: "tray" | "exit", remember: boolean) => {
    setShowCloseConfirm(false);
    if (remember) {
      localStorage.setItem("remember_close_action", action);
    }
    if (action === "tray") {
      await appWindow?.hide();
    } else {
      await invoke("app_exit");
    }
  };

  useEffect(() => {
    if (!settings.clipboard_monitor) return;
    let lastText = "";
    const initClipboard = async () => {
      try {
        const text = await readText();
        lastText = text;
      } catch (e) {}
    };
    void initClipboard();

    const interval = setInterval(async () => {
      try {
        const text = await readText();
        if (text && text !== lastText) {
          lastText = text;
          const match = text.match(/https?:\/\/[^\s<>"']+/i);
          if (match) {
            const url = match[0];
            if (isDownloadableUrl(url)) {
              setInitialUrlFromClipboard(url);
              setNewOpen(true);
              if (appWindow) {
                await appWindow.show();
                await appWindow.unminimize();
                await appWindow.setFocus();
              }
            }
          }
        }
      } catch (e) {}
    }, 1500);
    return () => clearInterval(interval);
  }, [settings.clipboard_monitor]);

  // Task 22.3：详情栏默认折叠/展开开关。
  // - 切换选中任务时（taskId 变化），按 detail_default_collapsed 决定默认状态。
  // - 选择数从 0 增长时也按该设置决定首次是否展开。
  // - 用户手动展开/折叠后保持当前任务内状态，直到切换任务。
  // - 显式请求（"查看详情"/"定位任务"按钮）通过 requestShowDetails 标记跳过自动折叠。
  const lastSelectedTaskId = useRef<string | undefined>(undefined);
  const lastSelectedCount = useRef(0);
  const skipAutoCollapseRef = useRef(false);
  const requestShowDetails = (value: boolean) => {
    skipAutoCollapseRef.current = true;
    setShowDetails(value);
  };


  // Task 24: 按归档规则将任务拆分为主列表与历史视图。
  // - 已完成且 (now - completed_at) > archive_days 天 → 历史
  // - 主列表已完成任务数 > archive_threshold 时，最旧的若干条 → 历史
  // 其余任务留在主列表。
  const partitioned = useMemo(() => {
    const archiveMs = Math.max(0, settings.archive_days) * 86_400_000;
    const threshold = Math.max(0, settings.archive_threshold);
    const now = Date.now();
    const isOldCompleted = (task: DownloadTask) =>
      task.status === "completed"
      && task.completed_at != null
      && archiveMs > 0
      && now - task.completed_at > archiveMs;
    // 主列表候选 = 排除"超期已完成"后剩余的已完成任务，按完成时间升序（最旧在前）。
    const mainCompletedSorted = tasks
      .filter((task) => task.status === "completed" && !isOldCompleted(task))
      .sort((a, b) => (a.completed_at ?? 0) - (b.completed_at ?? 0));
    const overflowCount = Math.max(0, mainCompletedSorted.length - threshold);
    const overflowIds = new Set(
      mainCompletedSorted.slice(0, overflowCount).map((task) => task.id),
    );
    const mainTasks: DownloadTask[] = [];
    const historyTasks: DownloadTask[] = [];
    for (const task of tasks) {
      if (isOldCompleted(task) || overflowIds.has(task.id)) {
        historyTasks.push(task);
      } else {
        mainTasks.push(task);
      }
    }
    return { mainTasks, historyTasks };
  }, [tasks, settings.archive_days, settings.archive_threshold]);

  const visible = useMemo(() => {
    // Task 24: 历史视图忽略主列表筛选规则，使用独立状态 historyStatusFilter；
    // 主列表保持原有 nav/categories 筛选行为。
    // Task 25: 主列表额外应用 advancedFilter（状态/域名/日期/大小/标签/来源）；
    // 历史视图不受 advancedFilter 影响（已有自己的筛选维度）。
    const source = view === "history" ? partitioned.historyTasks : partitioned.mainTasks;
    return source.filter((task) => {
      if (view === "history") {
        const statusOk = historyStatusFilter === "all" || task.status === historyStatusFilter;
        const date = matchesHistoryDate(task.completed_at, historyDate);
        return statusOk && date && `${task.file_name} ${task.url}`.toLowerCase().includes(search.toLowerCase());
      }
      const category = categories.some(([key]) => key === filter) ? task.category === filter : true;
      const status = nav.some(([key]) => key === filter && key !== "all") ? task.status === filter : true;
      const date = filter !== "completed" || matchesHistoryDate(task.completed_at, historyDate);
      const searchOk = `${task.file_name} ${task.url}`.toLowerCase().includes(search.toLowerCase());
      const advancedOk = matchesAdvancedFilter(task, advancedFilter, taskTags[task.id] ?? []);
      return category && status && date && searchOk && advancedOk;
    }).sort((a, b) => {
      const av = a[sort.key] ?? ""; const bv = b[sort.key] ?? "";
      const result = typeof av === "number" && typeof bv === "number" ? av - bv : String(av).localeCompare(String(bv));
      return sort.desc ? -result : result;
    });
  }, [partitioned, view, filter, historyDate, search, sort, historyStatusFilter, advancedFilter, taskTags]);
  const selectedTasks = tasks.filter((task) => selected.has(task.id));
  const selectedOne = selectedTasks.length === 1 ? selectedTasks[0] : undefined;
  const activeTask = useMemo(() => {
    if (selected.size === 0) return undefined;
    if (primaryTaskId && selected.has(primaryTaskId)) {
      const found = tasks.find((t) => t.id === primaryTaskId);
      if (found) return found;
    }
    return selectedTasks.length > 0 ? selectedTasks[selectedTasks.length - 1] : undefined;
  }, [selected, primaryTaskId, tasks, selectedTasks]);

  useEffect(() => {
    const currentCount = selected.size;
    const currentTaskId = activeTask?.id;
    const taskIdChanged = currentTaskId !== lastSelectedTaskId.current;
    if (currentCount === 0) {
      setShowDetails(false);
      skipAutoCollapseRef.current = false;
      setPrimaryTaskId(undefined);
    } else if (taskIdChanged) {
      if (skipAutoCollapseRef.current) {
        // 用户已显式请求展开/折叠，保持其意图，本次切换不覆盖。
        skipAutoCollapseRef.current = false;
      } else {
        // 切换选中任务时按设置决定默认展开/折叠。
        setShowDetails(!settings.detail_default_collapsed);
      }
    }
    // 同一任务内增减选择（多选/取消多选）不重置 showDetails。
    lastSelectedCount.current = currentCount;
    lastSelectedTaskId.current = currentTaskId;
  }, [selected, activeTask, settings.detail_default_collapsed]);
  const active = tasks.filter((task) => task.status === "downloading");
  const totalSpeed = active.reduce((sum, task) => sum + task.speed, 0);
  const notify = (text: string, kind: "ok" | "error" = "ok") => setToast({ text, kind });
  // notifyRef / refreshRef 始终指向最新的函数
  notifyRef.current = notify;
  refreshRef.current = refresh;
  const armPowerAction = async (action: Exclude<PowerAction, "none">) => {
    try {
      setPowerAction(await api.armPowerAction(action));
      notify(action === "shutdown" ? "已设置队列完成后关机" : "已设置队列完成后休眠");
    } catch (error) {
      notify(String(error), "error");
      throw error;
    }
  };
  const cancelPowerAction = async () => {
    try {
      setPowerAction(await api.cancelPowerAction());
      notify("已取消队列完成后的系统操作");
    } catch (error) { notify(String(error), "error"); }
  };
  const bulk = async (action: string) => {
    try {
      const ids = action === "resume"
        ? [...selected].filter((id) => { const t = tasks.find((task) => task.id === id); return t && !["completed", "cancelled"].includes(t.status); })
        : [...selected];
      if (ids.length === 0) return;
      await api.bulkAction(ids, action);
      notify(action === "pause" ? "已暂停所选任务" : "任务已加入队列");
    } catch (error) { notify(String(error), "error"); }
  };
  const removeSelected = async (deleteFile: boolean) => {
    try {
      const selectedList = tasks.filter((t) => selected.has(t.id));
      const hasIncomplete = selectedList.some((t) => t.status !== "completed");
      for (const id of selected) await api.remove(id, deleteFile);
      setSelected(new Set());
      notify(deleteFile ? "任务和文件已删除" : hasIncomplete ? "任务记录及未完成文件已清理" : "任务记录已删除");
    } catch (error) { notify(String(error), "error"); }
  };
  // Task 24.3: 清空历史视图——仅删除当前归入历史的任务，不动主列表。
  const clearHistory = async (deleteFile: boolean) => {
    try {
      for (const task of partitioned.historyTasks) await api.remove(task.id, deleteFile);
      setSelected(new Set());
      notify(deleteFile ? `已删除 ${partitioned.historyTasks.length} 个历史任务及文件` : `已删除 ${partitioned.historyTasks.length} 个历史任务记录`);
    } catch (error) { notify(String(error), "error"); }
  };
  // Task 16.3: 拖拽排序——直接操作 DOM class 高亮落点，零 setState 调用。
  // 重排成功后不调用 setSort（防止 WebView2 DOM 重排后鼠标事件失效），改为调用 refreshRef 让后端事件自然推送更新。
  const handleTaskMouseDown = useCallback((task: DownloadTask, event: React.MouseEvent) => {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    if (target.closest("input, button, label")) return;
    event.preventDefault();
    dragRef.current = { taskId: task.id, startY: event.clientY, active: false, hoverId: null, sourceEl: event.currentTarget as HTMLElement, dropEl: null };
    const handleMouseMove = (mv: globalThis.MouseEvent) => {
      const ref = dragRef.current;
      if (!ref) return;
      if (!ref.active) {
        if (Math.abs(mv.clientY - ref.startY) < 6) return;
        ref.active = true;
        ref.sourceEl.classList.add("dragging");
        document.body.style.cursor = "grabbing";
      }
      const el = document.elementFromPoint(mv.clientX, mv.clientY);
      const rowEl = el?.closest<HTMLElement>(".task-row");
      const hoverId = rowEl && rowEl.dataset.taskId !== ref.taskId ? rowEl.dataset.taskId ?? null : null;
      if (ref.dropEl && ref.dropEl !== rowEl) {
        ref.dropEl.classList.remove("drop-target");
        ref.dropEl = null;
      }
      if (rowEl && hoverId) {
        rowEl.classList.add("drop-target");
        ref.dropEl = rowEl;
      }
      ref.hoverId = hoverId;
    };
    const handleMouseUp = async () => {
      window.removeEventListener("mousemove", handleMouseMove, true);
      window.removeEventListener("mouseup", handleMouseUp, true);
      document.body.style.cursor = "";
      const ref = dragRef.current;
      dragRef.current = null;
      ref?.sourceEl.classList.remove("dragging");
      if (ref?.dropEl) { ref.dropEl.classList.remove("drop-target"); }
      if (!ref?.active) return;
      const targetId = ref.hoverId;
      if (!targetId || targetId === ref.taskId) return;
      const currentTasks = tasksRef.current;
      const dragged = currentTasks.find((t) => t.id === ref.taskId);
      const dropTarget = currentTasks.find((t) => t.id === targetId);
      if (!dragged || !dropTarget) return;
      if (dragged.priority !== dropTarget.priority) {
        notifyRef.current("请通过右键菜单或数字优先级调整跨优先级排序", "error");
        return;
      }
      const reorderedIds = reorderTaskIdsWithinPriority(currentTasks, dragged.id, dropTarget.id);
      if (!reorderedIds) return;
      try {
        await api.reorder(reorderedIds);
        const positions = new Map(reorderedIds.map((id, index) => [id, index]));
        setTasks((items) => items.map((item) => {
          const position = positions.get(item.id);
          return position === undefined ? item : { ...item, queue_position: position };
        }));
        // 拖拽表达的是队列顺序；完成后切回队列视图，确保结果立即可见。
        setSort({ key: "queue_position", desc: false });
        void refreshRef.current();
        notifyRef.current("队列顺序已更新");
      } catch (error) {
        notifyRef.current(String(error), "error");
      }
    };
    window.addEventListener("mousemove", handleMouseMove, true);
    window.addEventListener("mouseup", handleMouseUp, true);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const beginResize = (key: string, event: MouseEvent) => {
    event.preventDefault(); event.stopPropagation(); const start = event.clientX;
    const defaultWidths: Record<string, number> = { size: 78, status: 82, connection: 64, progress: 130, speed: 78, eta: 82, created: 100 };
    const isSmallScreen = window.innerWidth <= 1180;
    const fallbackWidth = isSmallScreen 
      ? { size: 70, status: 82, connection: 58, progress: 112, speed: 72, eta: 76, created: 92 }[key] ?? 70
      : defaultWidths[key] ?? 80;
    const width = columnWidths[key] ?? fallbackWidth;
    const move = (next: globalThis.MouseEvent) => setColumnWidths((value) => ({ ...value, [key]: Math.max(58, width + next.clientX - start) }));
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move); window.addEventListener("mouseup", up);
  };
  const handleDeleteTasks = useCallback(async (taskIds: Set<string>, deleteFile: boolean) => {
    if (taskIds.size === 0) return;
    const taskList = tasks.filter((t) => taskIds.has(t.id));
    const hasIncomplete = taskList.some((t) => t.status !== "completed");
    const succeeded: string[] = [];
    try {
      for (const id of taskIds) {
        await api.remove(id, deleteFile);
        succeeded.push(id);
      }
      const count = succeeded.length;
      notify(
        deleteFile
          ? count === 1 ? "任务和文件已删除" : `${count} 个任务和文件已删除`
          : hasIncomplete
          ? count === 1 ? "任务及未完成文件已清理" : `${count} 个任务及未完成文件已清理`
          : count === 1 ? "任务记录已删除" : `${count} 个任务记录已删除`
      );
    } catch (error) {
      notify(String(error), "error");
    } finally {
      if (succeeded.length > 0) {
        setSelected((prev) => {
          const next = new Set(prev);
          for (const id of succeeded) next.delete(id);
          return next;
        });
        setPrimaryTaskId((prev) => prev && succeeded.includes(prev) ? undefined : prev);
      }
    }
  }, [notify, tasks]);

  // Task 21.1: 任务列表全局快捷键。
  // - 仅在主列表区域使用：当焦点在 input/textarea/contenteditable 时不触发，
  //   避免劫持输入框内的全选文本、空格等默认行为。
  // - 支持从 settings.shortcut_keys 动态匹配绑定的快捷键组合。
  useEffect(() => {
    const isEditing = () => {
      const active = document.activeElement as HTMLElement | null;
      if (!active) return false;
      const tag = active.tagName;
      return tag === "INPUT" || tag === "TEXTAREA" || active.isContentEditable;
    };
    const handler = (event: KeyboardEvent) => {
      // 弹窗打开时也不触发列表快捷键（避免与对话框内输入冲突）
      if (newOpen || settingsOpen || renameTarget || speedLimitTarget || aboutOpen || showCloseConfirm || context) return;
      if (isEditing()) return;

      const keys = settings.shortcut_keys || DEFAULT_SHORTCUTS;

      // 新建任务
      if (matchesShortcut(event, keys.new_task)) {
        event.preventDefault();
        setNewOpen(true);
        return;
      }
      // 全选 / 取消全选（连续按两下在全选和清空之间切换）
      if (matchesShortcut(event, keys.select_all)) {
        if (visible.length === 0) return;
        event.preventDefault();
        const allSelected = visible.length > 0 && selected.size === visible.length && visible.every((t) => selected.has(t.id));
        if (allSelected) {
          setSelected(new Set());
        } else {
          setSelected(new Set(visible.map((task) => task.id)));
        }
        return;
      }
      // 复制来源 URL（单选或多选都复制）
      if (matchesShortcut(event, keys.copy_url)) {
        if (selectedTasks.length === 0) return;
        event.preventDefault();
        const text = selectedTasks.map((task) => task.url).join("\n");
        void navigator.clipboard.writeText(text)
          .then(() => notify(`已复制 ${selectedTasks.length} 个来源 URL`))
          .catch((error) => notify(`复制 URL 失败：${String(error)}`, "error"));
        return;
      }
      // 打开所在文件夹（仅单选 + Completed 状态）
      if (matchesShortcut(event, keys.open_folder)) {
        if (!selectedOne || selectedOne.status !== "completed") return;
        event.preventDefault();
        void api.openFolder(selectedOne.id).catch((error) => notify(String(error), "error"));
        return;
      }
      // 删文件（连同本地文件一并删除）
      if (matchesShortcut(event, keys.delete_file)) {
        if (selected.size === 0) return;
        event.preventDefault();
        void handleDeleteTasks(new Set(selected), true);
        return;
      }
      // 仅删除任务记录（无需确认）
      if (matchesShortcut(event, keys.delete_task)) {
        if (selected.size === 0) return;
        event.preventDefault();
        void handleDeleteTasks(new Set(selected), false);
        return;
      }
      // 重命名（仅单选 + Queued/Pending 状态，弹出重命名对话框）
      if (matchesShortcut(event, keys.rename_task)) {
        if (!selectedOne) return;
        event.preventDefault();
        if (selectedOne.status !== "queued") {
          notify("任务已开始，无法重命名", "error");
          return;
        }
        setRenameTarget(selectedOne);
        return;
      }
      // 暂停/继续选中任务（单选时；多选时对全部生效）
      if (matchesShortcut(event, keys.toggle_pause) && !event.repeat) {
        if (selected.size === 0) return;
        event.preventDefault();
        const anyActive = tasks.some((task) => selected.has(task.id) && ["downloading", "waiting-network"].includes(task.status));
        void bulk(anyActive ? "pause" : "resume");
        return;
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selected, tasks, visible, selectedTasks, selectedOne, newOpen, settingsOpen, renameTarget, speedLimitTarget, aboutOpen, showCloseConfirm, context, view, handleDeleteTasks, settings.shortcut_keys, notify, bulk]);

  const titlebar = isDesktop() ? <Titlebar /> : null;

  if (settingsOpen) return (
    <div className="app-container">
      {titlebar}
      <SettingsPage value={settings} onChange={setSettings} onClose={() => setSettingsOpen(false)} notify={notify} totalSpeed={totalSpeed} activeCount={active.length} />
      <WindowResizeHandles />
      {toast && <div className="toast"><span>{toast.kind === "ok" ? <Check size={14} /> : <AlertCircle size={14} />}</span>{toast.text}</div>}
      {showCloseConfirm && <CloseConfirmDialog onClose={() => setShowCloseConfirm(false)} onConfirm={handleCloseConfirm} />}
    </div>
  );
  const sectionTitle = view === "history"
    ? t("nav.history")
    : ([...getNav(), ...getCategories()].find(([key]) => key === filter)?.[1] ?? t("nav.allTasks"));
  // Task 24: 历史视图始终展示完成时间列；主列表仅在 completed 筛选下展示。
  const showCompletedAt = view === "history" || filter === "completed";
  return (
    <div className="app-container">
      {titlebar}
      <div className="app-frame">
        <aside className="nav-pane">
          <div className="brand" onClick={() => setAboutOpen(true)} title={t("app.about")}><div className="app-icon"><CatDownloadMark /></div><span><b>{t("app.name")}</b><small>{t("app.nameEn")}</small></span></div>
          <button className="new-button" onClick={() => setNewOpen(true)}><Plus size={15} />{t("nav.newTask")}</button>
          <div className="nav-scroll">
            <p className="nav-label">{t("nav.tasks")}</p>
            {getNav().map(([key, label, Icon]) => <button key={key} className={filter === key && view === "main" ? "nav-item active" : "nav-item"} onClick={() => { setView("main"); setFilter(key); setSelected(new Set()); setPrimaryTaskId(undefined); }}><Icon size={14} /><span>{label}</span><small>{key === "all" ? partitioned.mainTasks.length : partitioned.mainTasks.filter((task) => task.status === key).length}</small></button>)}
            <p
              className="nav-label interactive"
              onClick={() => setCategoriesExpanded(!categoriesExpanded)}
            >
              <span>{t("nav.types")}</span>
              <span className={`nav-label-chevron ${categoriesExpanded ? "" : "collapsed"}`}>
                <ChevronDown size={12} />
              </span>
            </p>
            {categoriesExpanded && (
              <div className="nav-grid">
                {getCategories().map(([key, label, Icon]) => <button key={key} className={filter === key && view === "main" ? "nav-item active" : "nav-item"} onClick={() => { setView("main"); setFilter(key); setSelected(new Set()); setPrimaryTaskId(undefined); }}><Icon size={14} /><span>{label}</span><small>{partitioned.mainTasks.filter((task) => task.category === key).length || ""}</small></button>)}
              </div>
            )}
            <p className="nav-label">{t("nav.archive")}</p>
            <button className={view === "history" ? "nav-item active" : "nav-item"} onClick={() => { setView("history"); setSelected(new Set()); setPrimaryTaskId(undefined); }} title={t("nav.historyArchive")}><Archive size={14} /><span>{t("nav.history")}</span><small>{partitioned.historyTasks.length || ""}</small></button>
          </div>
          <div className="nav-footer">
            <button className="nav-settings" onClick={() => setSettingsOpen(true)}><Settings size={15} /><span>{t("nav.settings")}</span></button>
            <div className="nav-status" onClick={() => setSettingsOpen(true)}>
              <i className={isDesktop() ? "status-dot online" : "status-dot offline"} />
              <span>{t("nav.speedFormat", { speed: `${formatBytes(totalSpeed)}/s`, count: active.length })}</span>
            </div>
          </div>
        </aside>
        <main className="workspace">
          <header className="titlebar" data-tauri-drag-region>
            <h1 data-tauri-drag-region>{sectionTitle}</h1>
            <label className="search-box"><Search size={14} /><input aria-label={t("toolbar.searchAria")} value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("toolbar.searchPlaceholder")} />{search && <button onClick={() => setSearch("")}><X size={13} /></button>}</label>
            <div className="toolbar-actions">
              <button className="action-btn-standalone" onClick={() => setNewOpen(true)} title={t("toolbar.newTask")}><Plus size={14} /></button>

              {view === "main" && (
                <button
                  className={`action-btn-standalone${advancedFilterOpen ? " active" : ""}`}
                  onClick={() => setAdvancedFilterOpen((v) => !v)}
                  title={t("toolbar.advancedFilter")}
                  aria-pressed={advancedFilterOpen}
                >
                  <ListFilter size={14} />
                  {!isAdvancedFilterEmpty(advancedFilter) && <span className="filter-badge" aria-label={t("toolbar.filterApplied")} />}
                </button>
              )}

              <div className="action-group">
                <button disabled={!selected.size} onClick={() => void bulk("resume")} title={t("toolbar.startTask")}><Play size={14} /></button>
                <button disabled={!selected.size} onClick={() => void bulk("pause")} title={t("toolbar.pauseTask")}><Pause size={14} /></button>
                <button className="danger-action" disabled={!selected.size} onClick={() => void removeSelected(false)} title={t("toolbar.deleteRecord")}><Trash2 size={14} /></button>
              </div>

              <div className="action-group">
                <button disabled={!selectedOne || selectedOne.status !== "completed"} onClick={() => selectedOne && void api.openFile(selectedOne.id)} title={t("toolbar.openFile")}><ExternalLink size={14} /></button>
                <button disabled={!selectedOne} onClick={() => selectedOne && void api.openFolder(selectedOne.id)} title={t("toolbar.openFolder")}><FolderOpen size={14} /></button>
              </div>

              {view === "history" && (
                <button
                  className="action-btn-standalone danger-action"
                  disabled={partitioned.historyTasks.length === 0}
                  onClick={() => void clearHistory(true)}
                  title={t("toolbar.clearHistory")}
                ><Trash2 size={14} /></button>
              )}

              <button className="action-btn-standalone" onClick={() => void refresh()} title={t("toolbar.refreshList")}><RefreshCw size={14} /></button>
              <PowerActionButton state={powerAction} onArm={armPowerAction} onCancel={cancelPowerAction} />
            </div>
            <button className="details-toggle" onClick={() => setShowDetails((value) => !value)} title={t("toolbar.detailsPanel")}>{showDetails ? <PanelRightClose size={15} /> : <PanelRightOpen size={15} />}</button>
          </header>
          {fatal && <div className="error-banner"><Unplug size={16} /><span>{t("toasts.kernelConnectionFailed", { error: fatal })}</span><button onClick={() => void refresh()}>{t("common.retry")}</button></div>}
          <PowerActionBanner state={powerAction} onCancel={cancelPowerAction} />
          {view === "history" ? (
            <div className="history-filter-bar" aria-label={t("historyFilter.status")}>
              <span>{t("historyFilter.status")}</span>
              <Select
                value={historyStatusFilter}
                onChange={(val: any) => setHistoryStatusFilter(val as TaskStatus | "all")}
                options={[
                  { value: "all", label: t("historyFilter.allStatuses") },
                  { value: "completed", label: t("status.completed") },
                  { value: "failed", label: t("status.failed") },
                  { value: "cancelled", label: t("status.cancelled") },
                  { value: "interrupted", label: t("status.interrupted") },
                ]}
                ariaLabel={t("historyFilter.status")}
              />
              <span className="history-filter-separator" aria-hidden="true">·</span>
              <span>{t("historyFilter.completionDate")}</span>
              <Select
                value={historyDate.preset}
                onChange={(val: any) => setHistoryDate({ ...historyDate, preset: val as typeof historyDate.preset })}
                options={[
                  { value: "all", label: t("historyFilter.allTime") },
                  { value: "today", label: t("historyFilter.today") },
                  { value: "7-days", label: t("historyFilter.last7Days") },
                  { value: "30-days", label: t("historyFilter.last30Days") },
                  { value: "custom", label: t("historyFilter.customRange") },
                ]}
                ariaLabel={t("historyFilter.completionDate")}
              />
              {historyDate.preset === "custom" && <>
                <input type="date" aria-label={t("historyFilter.startDate")} value={historyDate.start} onChange={(event) => setHistoryDate({ ...historyDate, start: event.target.value })} />
                <span>{t("historyFilter.to")}</span>
                <input type="date" aria-label={t("historyFilter.endDate")} value={historyDate.end} min={historyDate.start || undefined} onChange={(event) => setHistoryDate({ ...historyDate, end: event.target.value })} />
              </>}
            </div>
          ) : (filter === "completed" && <HistoryDateFilter value={historyDate} onChange={setHistoryDate} />)}
          {view === "main" && advancedFilterOpen && (
            <AdvancedFilterPanel
              value={advancedFilter}
              onChange={setAdvancedFilter}
              tags={tags}
              quickViews={quickViews}
              onApplyQuickView={(qv) => setAdvancedFilter({ ...qv.filter })}
              onSaveQuickView={(name) => setQuickViews((current) => [...current, { id: newQuickViewId(), name, filter: advancedFilter }])}
              onDeleteQuickView={(id) => setQuickViews((current) => current.filter((qv) => qv.id !== id))}
              onClear={() => setAdvancedFilter({ ...EMPTY_ADVANCED_FILTER })}
            />
          )}
          <section className={showDetails ? "content-grid details-on" : "content-grid"}>
            <div
              className="task-list-panel"
              style={
                Object.fromEntries(
                  Object.entries(columnWidths)
                    .filter(([_, v]) => v !== undefined)
                    .map(([k, v]) => [`--col-${k}`, `${v}px`])
                ) as CSSProperties
              }
            >
              <div className="task-grid">
              <div className="table-header"><label><input type="checkbox" aria-label={t("toolbar.selectAll")} checked={visible.length > 0 && visible.every((task) => selected.has(task.id))} onChange={() => setSelected(visible.every((task) => selected.has(task.id)) ? new Set() : new Set(visible.map((task) => task.id)))} /></label>{[["file_name",t("table.fileName"),""],["total_bytes",t("table.size"),"size"],["status",t("table.status"),"status"],["connection_count",t("table.connection"),"connection"],["downloaded_bytes",t("table.progress"),"progress"],["speed",t("table.speed"),"speed"],["eta_seconds",t("table.eta"),"eta"],[showCompletedAt ? "completed_at" : "created_at",showCompletedAt ? t("table.completedAt") : t("table.createdAt"),"created"]].map(([key,label,widthKey]) => <span key={key} onClick={() => setSort((current) => ({ key: key as keyof DownloadTask, desc: current.key === key ? !current.desc : ["created_at", "completed_at"].includes(key) }))}>{label}{widthKey && <i className="column-resizer" onMouseDown={(event) => beginResize(widthKey, event)} />}</span>)}<span /></div>
              <div className="task-rows">{loading ? <div className="center-state"><LoaderCircle className="spin" /></div> : visible.length === 0 ? <EmptyState filter={filter} view={view} onAdd={() => setNewOpen(true)} /> : visible.map((task) => <TaskRow key={task.id} task={task} showCompletedAt={showCompletedAt} taskTagList={taskTags[task.id] ?? []} selected={selected.has(task.id)} onSelect={() => { setPrimaryTaskId(task.id); setSelected((current) => { const next = new Set(current); next.has(task.id) ? next.delete(task.id) : next.add(task.id); return next; }); }} onOpen={() => task.status === "completed" && void api.openFile(task.id)} onContext={(event) => { event.preventDefault(); setPrimaryTaskId(task.id); setContext({ x: event.clientX, y: event.clientY, id: task.id }); if (!selected.has(task.id)) setSelected(new Set([task.id])); }} onMouseDown={(taskItem, evt) => { setPrimaryTaskId(taskItem.id); handleTaskMouseDown(taskItem, evt); }} onCheckboxMouseDown={(evt) => handleCheckboxMouseDown(task.id, selected.has(task.id), evt)} onCheckboxMouseEnter={() => handleCheckboxMouseEnter(task.id)} />)}</div>
            </div></div>
            {showDetails && <Details task={activeTask} onClose={() => setShowDetails(false)} notify={notify} selectedCount={selected.size} onOpenProxySettings={() => { setSettingsOpen(true); }} onOpenYouTubeModal={() => setYoutubeModalTaskId(activeTask?.id || "")} onTagsChanged={refreshTags} />}
          </section>
        </main>
        {newOpen && <NewTaskDialog settings={settings} allTasks={tasks} onClose={() => { setNewOpen(false); setInitialUrlFromClipboard(""); }} onCreated={(created) => {
          setNewOpen(false);
          setInitialUrlFromClipboard("");
          const list = Array.isArray(created) ? created : [created];
          notify(t("toasts.addedTasks", { count: list.length }));
          if (list.length > 0) {
            setSelected(new Set(list.map((t) => t.id)));
          }
        }} defaultUrl={initialUrlFromClipboard} onLocateTask={(taskId) => {
          setNewOpen(false);
          setInitialUrlFromClipboard("");
          setSelected(new Set([taskId]));
          // Task 22.3：用户显式定位任务后请求展开详情，跳过 detail_default_collapsed 自动折叠。
          requestShowDetails(true);
          setFilter("all");
        }} notify={notify} />}
        {(() => {
          const contextTask = context ? tasks.find((t) => t.id === context.id) : undefined;
          return context && contextTask ? (
            <ContextMenu
              x={context.x}
              y={context.y}
              task={contextTask}
              selectedTaskIds={selected}
              allTasks={tasks}
              close={() => setContext(undefined)}
              notify={notify}
              onSetSpeedLimit={setSpeedLimitTarget}
              onDelete={(taskIds, deleteFile) => void handleDeleteTasks(taskIds, deleteFile)}
              onViewDetails={() => {
                setPrimaryTaskId(contextTask.id);
                if (!selected.has(contextTask.id)) {
                  setSelected(new Set([contextTask.id]));
                }
                requestShowDetails(true);
                setContext(undefined);
              }}
            />
          ) : null;
        })()}
        {speedLimitTarget && (
          <SpeedLimitDialog
            task={speedLimitTarget}
            onClose={() => setSpeedLimitTarget(null)}
            onConfirm={async (limitKb) => {
              await api.updateTaskOptions(speedLimitTarget.id, { perTaskSpeedLimit: limitKb * 1024 });
              notify("限速已更新");
            }}
          />
        )}
        {toast && <div className="toast"><span>{toast.kind === "ok" ? <Check size={14} /> : <AlertCircle size={14} />}</span>{toast.text}</div>}
        {selfcheckToast && (
          <div className="toast toast-with-action" role="status">
            <span className="toast-icon"><AlertTriangle size={14} /></span>
            <div className="toast-body">
              <span>{t("toasts.recoveredInterrupted", { count: selfcheckToast.interrupted, dropped: selfcheckToast.dropped > 0 ? t("toasts.recoveredDroppedShards", { count: selfcheckToast.dropped }) : "" })}</span>
              <button
                className="toast-action-btn"
                onClick={() => {
                  const ids = selfcheckToast.taskIds;
                  setSelfcheckToast(undefined);
                  if (ids.length > 0) {
                    setSelected(new Set(ids));
                    // Task 22.3：用户显式点击"查看详情"，跳过 detail_default_collapsed 自动折叠。
                    requestShowDetails(true);
                    setFilter("all");
                  } else {
                    setFilter("failed");
                  }
                }}
              >
                <ListFilter size={11} />
                {t("toasts.viewDetails")}
              </button>
            </div>
            <button className="toast-close-btn" onClick={() => setSelfcheckToast(undefined)} aria-label={t("common.close")}>
              <X size={11} />
            </button>
          </div>
        )}
        {failureToast && (
          <div className="toast toast-with-action" role="alert">
            <span className="toast-icon"><AlertCircle size={14} /></span>
            <div className="toast-body">
              <span>{failureToast.title}</span>
              <span className="toast-subtext">{failureToast.body}</span>
              <div className="toast-actions">
                <button
                  className="toast-action-btn"
                  onClick={async () => {
                    const taskId = failureToast.taskId;
                    setFailureToast(undefined);
                    try {
                      await api.action(taskId, "retry");
                      setSelected(new Set([taskId]));
                      requestShowDetails(true);
                      setFilter("all");
                    } catch (error) {
                      notify(String(error), "error");
                    }
                  }}
                >
                  <RefreshCw size={11} />
                  {t("toasts.retryNow")}
                </button>
                <button
                  className="toast-action-btn toast-action-btn-secondary"
                  onClick={() => {
                    const taskId = failureToast.taskId;
                    setFailureToast(undefined);
                    setSelected(new Set([taskId]));
                    requestShowDetails(true);
                    setFilter("all");
                  }}
                >
                  {t("toasts.viewDetails")}
                </button>
                {failureToast.body.includes("YouTube") && (
                  <button
                    className="toast-action-btn toast-action-btn-secondary"
                    onClick={() => {
                      const taskId = failureToast.taskId;
                      setFailureToast(undefined);
                      setYoutubeModalTaskId(taskId);
                    }}
                  >
                    <ShieldCheck size={11} />
                    同步/配置凭证
                  </button>
                )}
              </div>
            </div>
            <button className="toast-close-btn" onClick={() => setFailureToast(undefined)} aria-label={t("common.close")}>
              <X size={11} />
            </button>
          </div>
        )}
        {youtubeModalTaskId !== null && (
          <YouTubeCredentialsModal
            taskId={youtubeModalTaskId || undefined}
            onClose={() => setYoutubeModalTaskId(null)}
            notify={notify}
            onSuccessRetry={() => void refreshRef.current()}
          />
        )}
        {showCloseConfirm && <CloseConfirmDialog onClose={() => setShowCloseConfirm(false)} onConfirm={handleCloseConfirm} />}
        {aboutOpen && (
          <Modal title={t("app.about")} onClose={() => setAboutOpen(false)} style={{ width: "290px" }}>
            <div className="about-dialog-content">
              <div className="about-logo"><CatDownloadMark /></div>
              <h3>{t("app.nameFull")}</h3>
              <p className="about-version">{t("app.versionNumber")}</p>
              <p className="about-desc">
                {t("app.aboutDescLine1")}<br />
                {t("app.aboutDescLine2")}
              </p>
              <div className="about-links">
                <button className="about-link-btn" onClick={() => void openUrl("https://github.com/maobukeai/maobu-fetch")}>
                  <ExternalLink size={10} />
                  <span>{t("app.projectHome")}</span>
                </button>
              </div>
              <p className="about-copyright">{t("app.copyright")}</p>
            </div>
          </Modal>
        )}



        {renameTarget && (
          <RenameDialog
            task={renameTarget}
            onClose={() => setRenameTarget(null)}
            onRenamed={(newName) => {
              notify(t("toasts.renamedTo", { name: newName }));
              setRenameTarget(null);
            }}
          />
        )}
      </div>
      <WindowResizeHandles />
      {splash && (
        <div id="splash-screen" className="splash-overlay">
          <div className="splash-content">
            <div className="splash-logo">
              <CatDownloadMark />
            </div>
            <div className="splash-brand">
              <strong className="splash-title">{t("app.name")}</strong>
              <span className="splash-subtitle">{t("app.nameEn")}</span>
            </div>
            <div className="splash-loader">
              <div className="splash-loader-bar" />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function isMediaTask(task: DownloadTask): boolean {
  if (task.media) return true;
  const mediaDomains = [
    "youtube.com", "youtu.be", "bilibili.com", "b23.tv", "douyin.com", "iesdouyin.com", "douyinvod.com",
    "vimeo.com", "tiktok.com", "twitter.com", "x.com", "weibo.com"
  ];
  try {
    const parsed = new URL(task.url);
    const hostname = parsed.hostname.toLowerCase();
    return mediaDomains.some(domain => hostname === domain || hostname.endsWith("." + domain));
  } catch {
    return false;
  }
}

function TaskRow({ task, selected, showCompletedAt, taskTagList, onSelect, onOpen, onContext, onMouseDown, onCheckboxMouseDown, onCheckboxMouseEnter }: { task: DownloadTask; selected: boolean; showCompletedAt: boolean; taskTagList: Tag[]; onSelect: () => void; onOpen: () => void; onContext: (event: MouseEvent) => void; onMouseDown: (task: DownloadTask, event: React.MouseEvent) => void; onCheckboxMouseDown: (event: React.MouseEvent) => void; onCheckboxMouseEnter: () => void }) {
  // Task 33: 订阅 locale 变化，语言切换时 TaskRow 重渲染以刷新状态文案。
  useLocale();
  const statusText = getStatusText();
  const progress = task.total_bytes ? Math.min(100, task.downloaded_bytes / task.total_bytes * 100) : 0;
  const speedMB = task.speed / (1024 * 1024);
  const stripeDuration = speedMB > 0 ? Math.max(0.25, Math.min(2.0, 1.5 / (speedMB + 0.5))) : 1.5;
  const isDownloading = ["downloading", "connecting", "verifying", "extracting"].includes(task.status);
  const canControl = ["downloading", "connecting", "verifying", "extracting", "paused", "failed", "cancelled"].includes(task.status);
  const handleAction = async (event: React.MouseEvent) => {
    event.stopPropagation();
    try {
      if (isDownloading) {
        await api.action(task.id, "pause");
      } else if (task.status === "failed") {
        await api.action(task.id, "retry");
      } else {
        await api.action(task.id, "resume");
      }
    } catch (e) {}
  };
  return <div
    className={selected ? "task-row selected" : "task-row"}
    data-task-id={task.id}
    onDoubleClick={onOpen}
    onContextMenu={onContext}
    onMouseDown={(e) => onMouseDown(task, e)}
  >
    <label
      onMouseDown={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onCheckboxMouseDown(e);
      }}
      onMouseEnter={onCheckboxMouseEnter}
      style={{ cursor: "pointer" }}
    >
      <input type="checkbox" aria-label={t("toolbar.selectAll")} checked={selected} readOnly />
    </label>
    <div className="name-cell" onClick={onSelect}>
      <FileIcon category={task.category} />
      <div style={{ minWidth: 0, flex: 1 }}>
        <div className="name-title-row">
          <strong title={task.file_name}>{task.file_name}</strong>
          {taskTagList.length > 0 && <TaskTagChips tags={taskTagList} max={3} />}
        </div>
        <small title={task.url}>
          {task.priority < 0 ? t("details.highPriority") : task.priority > 0 ? t("details.lowPriority") : ""}
          {hostOf(task.url)}
        </small>
      </div>
    </div>
    <span>{task.total_bytes ? formatBytes(task.total_bytes) : task.downloaded_bytes ? formatBytes(task.downloaded_bytes) : "—"}</span>
    <span className={`task-status ${task.status}`}>
      {task.status === "downloading" && isMediaTask(task) && task.downloaded_bytes === 0 && task.active_connections === 0 && !task.error
        ? statusText.parsing
        : statusText[task.status]}
      {canControl && (
        <button
          className="task-status-btn"
          onClick={handleAction}
          title={isDownloading ? t("details.taskStatusPause") : t("details.taskStatusResume")}
        >
          {isDownloading ? <Pause size={10} strokeWidth={2.5} /> : <Play size={10} strokeWidth={2.5} />}
        </button>
      )}
    </span>
    <span className="connection-count">{task.status === "downloading" ? `${task.active_connections}/${task.connection_count}` : task.connection_count}<small> {t("table.connectionUnit")}</small></span>
    <div className="progress-cell">
      <div style={{ position: "relative", overflow: "visible", flex: 1, display: "flex", alignItems: "center" }}>
        <div style={{ flex: 1, height: "4px", overflow: "hidden", borderRadius: "2px", background: "var(--progress-track)", display: "flex" }}>
          <i style={{ width: `${progress}%`, "--stripe-duration": `${stripeDuration}s` } as CSSProperties} className={task.status === "downloading" && task.connection_count > 1 ? "multi-thread" : ""} />
        </div>
        {task.status === "downloading" && task.connection_count > 1 && (
          <span
            className="speed-up-icon"
            style={{ left: `calc(${progress}% - 6px)` }}
            title={t("details.multiThread", { count: task.active_connections })}
          >
            <Zap size={11} strokeWidth={2.5} />
          </span>
        )}
      </div>
      <span>{task.status === "completed" ? "100%" : `${progress.toFixed(0)}%`}</span>
    </div>
    <span>{task.status === "downloading" ? `${formatBytes(task.speed)}/s` : "—"}</span><span>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</span><span>{formatDate(showCompletedAt ? task.completed_at ?? task.created_at : task.created_at)}</span><button className="row-menu" onClick={(event) => { event.stopPropagation(); onContext(event); }}><MoreHorizontal size={15} /></button>
  </div>;
}
function CatDownloadMark() { return <svg viewBox="0 0 1024 1024" aria-hidden="true"><rect x="48" y="48" width="928" height="928" rx="220" fill="#f5f5f7" /><path d="M302 360 358 230l112 78c28-9 56-14 86-14s58 5 86 14l112-78 56 130v214c0 151-113 254-254 254S302 725 302 574V360Z" fill="#1d1d1f" /><path d="M556 392v218m-86-82 86 86 86-86" fill="none" stroke="#f5f5f7" strokeWidth="58" strokeLinecap="round" strokeLinejoin="round" /><path d="M445 694h222" fill="none" stroke="#0a84ff" strokeWidth="58" strokeLinecap="round" /><circle cx="428" cy="430" r="19" fill="#f5f5f7" /><circle cx="684" cy="430" r="19" fill="#f5f5f7" /><path d="M755 700c86 15 119-50 76-103" fill="none" stroke="#1d1d1f" strokeWidth="48" strokeLinecap="round" /></svg>; }
function FileIcon({ category }: { category: string }) { const Icon = category === "video" ? Film : category === "audio" ? FileAudio : category === "images" ? FileImage : category === "archives" ? Archive : category === "apps" ? File : FileText; return <span className={`file-type ${category}`}><Icon size={16} /></span>; }

/**
 * Task 25: 在任务行 name-cell 中显示标签 chip。
 * - 最多显示 `max` 个，超出显示 "+N"
 * - chip 背景使用 Tag.color，文字使用对比色（白色加深阴影保证可读）
 * - chip 同时显示标签名文字，符合 AGENTS.md §4"交互不能只依赖颜色"
 */
function TaskTagChips({ tags, max }: { tags: Tag[]; max: number }) {
  if (tags.length === 0) return null;
  const shown = tags.slice(0, max);
  const overflow = tags.length - shown.length;
  return (
    <div className="task-tag-chips" aria-label={`标签：${tags.map((t) => t.name).join(", ")}`}>
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
function EmptyState({ filter, view, onAdd }: { filter: FilterKey; view: "main" | "history"; onAdd: () => void }) {
  // Task 33: 订阅 locale 变化，语言切换时空状态文案同步刷新。
  useLocale();
  if (view === "history") {
    return <div className="empty-state"><Archive size={36} /><h2>{t("empty.noHistoryTasks")}</h2><p>{t("empty.noHistoryTasksDesc")}</p></div>;
  }
  return <div className="empty-state"><Download size={36} /><h2>{filter === "all" ? t("empty.noTasks") : t("empty.noTasksInCategory")}</h2><p>{t("empty.noTasksDesc")}</p><button onClick={onAdd}>{t("nav.newTask")}</button></div>;
}

const DIAGNOSIS_TAB_STATUSES: TaskStatus[] = ["failed", "interrupted", "remote-changed", "paused-by-low-disk"];

function Details({ task, onClose, notify, selectedCount, onOpenProxySettings, onOpenYouTubeModal, onTagsChanged }: { task?: DownloadTask; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void; selectedCount: number; onOpenProxySettings?: () => void; onOpenYouTubeModal?: () => void; onTagsChanged?: () => void }) {
  // Task 33: 订阅 locale 变化，语言切换时详情面板文案同步刷新。
  useLocale();
  const [showMore, setShowMore] = useState(false);
  const [tab, setTab] = useState<"info" | "diagnosis" | "precheck" | "connections">("info");
  const [precheck, setPrecheck] = useState<{
    loading: boolean;
    result?: PrecheckResult;
    error?: string;
  }>({ loading: false });

  const taskId = task?.id;
  const taskStatus = task?.status;

  const activeRequestRef = useRef<string | null>(null);

  // 当任务切换或状态变化时，重置标签页与预检缓存。
  useEffect(() => {
    activeRequestRef.current = null;
    setPrecheck({ loading: false });
    if (taskStatus && DIAGNOSIS_TAB_STATUSES.includes(taskStatus)) {
      setTab("diagnosis");
    } else {
      setTab("info");
    }
  }, [taskId, taskStatus]);

  const runPrecheck = async () => {
    if (!task) return;
    const reqId = Math.random().toString(36).slice(2);
    activeRequestRef.current = reqId;
    setPrecheck({ loading: true });

    try {
      const timeoutPromise = new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error("前端预检等待超时（20 秒），请重试或检查网络")), 20_000)
      );
      const result = await Promise.race([
        api.precheck({
          url: task.url,
          target_directory: task.destination,
          suggested_filename: task.file_name,
          headers: Object.keys(task.headers || {}).length > 0 ? task.headers : undefined,
          proxy_override: task.proxy_override,
          proxy_auth: task.proxy_auth,
        }),
        timeoutPromise,
      ]);
      if (activeRequestRef.current === reqId) {
        setPrecheck({ loading: false, result });
      }
    } catch (err) {
      if (activeRequestRef.current === reqId) {
        setPrecheck({ loading: false, error: String(err) });
      }
    }
  };

  // 首次切到预检结果标签页时自动加载。
  useEffect(() => {
    if (tab === "precheck" && task && !precheck.result && !precheck.loading && !precheck.error) {
      void runPrecheck();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, taskId, precheck.loading, precheck.result, precheck.error]);

  if (!task) {
    return <aside className="details-pane">
      <div className="details-header">
        <h2>{t("details.title")}</h2>
        <button onClick={onClose} title={t("common.close")}><X size={14} /></button>
      </div>
      <div className="details-scroll" style={{ justifyContent: "center", alignItems: "center", color: "var(--muted)", textAlign: "center", padding: "24px 16px", gap: "12px" }}>
        <Info size={32} strokeWidth={1.5} style={{ opacity: 0.4, marginBottom: "4px" }} />
        {selectedCount > 1 ? (
          <>
            <h3 style={{ fontSize: "12px", fontWeight: 600, color: "var(--text)", margin: 0 }}>{t("details.selectedCount", { count: selectedCount })}</h3>
            <p style={{ fontSize: "10px", margin: 0, lineHeight: 1.4 }}>{t("details.selectedCountDesc")}</p>
          </>
        ) : (
          <>
            <h3 style={{ fontSize: "12px", fontWeight: 600, color: "var(--text)", margin: 0 }}>{t("details.notSelected")}</h3>
            <p style={{ fontSize: "10px", margin: 0, lineHeight: 1.4 }}>{t("details.notSelectedDesc")}</p>
          </>
        )}
      </div>
    </aside>;
  }

  const action = async (value: string) => { try { await api.action(task.id, value); } catch (error) { notify(String(error), "error"); } };
  const completionLabel = completionActionLabel(task.completion_action);
  const priorityLabel = task.priority < 0 ? t("details.priorityHigh") : task.priority > 0 ? t("details.priorityLow") : t("details.priorityNormal");

  return <aside className="details-pane">
    <div className="details-header">
      <h2>
        {task.file_name}
        {selectedCount > 1 && (
          <span style={{ fontSize: "11px", fontWeight: 400, color: "var(--subtle)", marginLeft: "8px" }}>
            ({t("details.selectedCount", { count: selectedCount })})
          </span>
        )}
      </h2>
      <button onClick={onClose} title={t("common.close")}><X size={14} /></button>
    </div>
    <div className="details-tabs" role="tablist" aria-label={t("details.title")}>
      <button role="tab" aria-selected={tab === "info"} className={tab === "info" ? "active" : ""} onClick={() => setTab("info")}>{t("details.tabInfo")}</button>
      <button role="tab" aria-selected={tab === "diagnosis"} className={tab === "diagnosis" ? "active" : ""} onClick={() => setTab("diagnosis")}>{t("details.tabDiagnosis")}</button>
      <button role="tab" aria-selected={tab === "precheck"} className={tab === "precheck" ? "active" : ""} onClick={() => setTab("precheck")}>{t("details.tabPrecheck")}</button>
      <button role="tab" aria-selected={tab === "connections"} className={tab === "connections" ? "active" : ""} onClick={() => setTab("connections")}>{t("details.tabConnections")}</button>
    </div>
    <div className="details-scroll">
      {tab === "info" && (
        <DetailsInfoTab
          task={task}
          showMore={showMore}
          onToggleMore={() => setShowMore(v => !v)}
          notify={notify}
          action={action}
          onTagsChanged={onTagsChanged}
        />
      )}
      {tab === "diagnosis" && (
        <DiagnosisPanel
          taskId={task.id}
          status={task.status}
          notify={notify}
          onOpenProxySettings={onOpenProxySettings}
          onOpenYouTubeModal={onOpenYouTubeModal}
          onTaskChanged={() => { /* 任务事件流会自动更新列表，无需手动刷新 */ }}
        />
      )}
      {tab === "precheck" && (
        <PrecheckPanel
          result={precheck.result}
          loading={precheck.loading}
          error={precheck.error}
          onRefresh={() => void runPrecheck()}
          compact
        />
      )}
      {tab === "connections" && (
        <DetailsConnectionsTab task={task} />
      )}
    </div>
  </aside>;
}

const CONNECTION_STATE_LABEL: Record<ConnectionState, string> = {
  connecting: "连接中",
  downloading: "下载中",
  retrying: "重试中",
  completed: "已完成",
  failed: "失败",
  paused: "已暂停",
};

/** Task 33: DetailsConnectionsTab 内部按当前 locale 取连接状态文案。 */
function useConnectionStateLabel(): Record<ConnectionState, string> {
  // useLocale 触发组件重渲染，确保语言切换时连接状态文案同步刷新。
  useLocale();
  return getConnectionStateLabel();
}

/**
 * Task 18：连接级实时状态面板。
 *
 * 监听 `task-connections` 事件（每秒一次，仅在 Downloading 状态推送；
 * 离开 Downloading 时后端会推送一次最终状态，例如全部 Paused）。
 *
 * 数据来自后端 `SegmentRuntime` 原子量的真实采样，非模拟（AGENTS.md §3）。
 * 不引入轮询——全部由 Tauri 事件触发（AGENTS.md §8）。
 *
 * 任务切换时卸载旧监听器并按 segment_id 索引保留最新分片状态。
 */
function DetailsConnectionsTab({ task }: { task: DownloadTask }) {
  const [segments, setSegments] = useState<Record<string, SegmentStatus>>({});
  const [lastTimestamp, setLastTimestamp] = useState<number | undefined>();
  const taskId = task.id;

  useEffect(() => {
    setSegments({});
    setLastTimestamp(undefined);
    if (!isDesktop()) return;

    let cancelled = false;
    const unlisten: Array<() => void> = [];

    listen<TaskConnectionsEvent>("task-connections", (event) => {
      if (cancelled) return;
      if (event.payload.task_id !== taskId) return;
      const next: Record<string, SegmentStatus> = {};
      for (const seg of event.payload.segments) {
        next[seg.segment_id] = seg;
      }
      setSegments(next);
      setLastTimestamp(event.payload.timestamp);
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten.push(fn);
      }
    });

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [taskId]);

  // 任务未收到事件时，从 task.segments 派生初始展示（例如刚切换到该任务）。
  const list = Object.values(segments).sort((a, b) => Number(a.segment_id) - Number(b.segment_id));
  const displayList: SegmentStatus[] = list.length > 0
    ? list
    : task.segments.map((seg) => ({
        segment_id: String(seg.index),
        start_offset: seg.start_byte,
        downloaded_bytes: seg.downloaded_bytes,
        total_bytes: seg.end_byte - seg.start_byte + 1,
        speed: 0,
        state: seg.status === "completed" ? "completed" : seg.status === "failed" ? "failed" : "paused",
        retry_count: 0,
        error: null,
      }));

  const totalCount = displayList.length;
  const completedCount = displayList.filter((s) => s.state === "completed").length;
  const activeCount = displayList.filter((s) => s.state === "downloading" || s.state === "connecting" || s.state === "retrying").length;
  const failedCount = displayList.filter((s) => s.state === "failed").length;
  const totalDownloaded = displayList.reduce((sum, s) => sum + s.downloaded_bytes, 0);
  const totalBytes = displayList.reduce((sum, s) => sum + s.total_bytes, 0);
  const overallPercent = totalBytes > 0 ? Math.min(100, Math.round((totalDownloaded / totalBytes) * 100)) : 0;
  const live = task.status === "downloading";

  if (totalCount === 0) {
    return <div className="connections-empty">
      <Network size={28} strokeWidth={1.5} style={{ opacity: 0.4, marginBottom: 4 }} />
      <h3>暂无分片连接</h3>
      <p>该任务未启用多连接，或尚未开始下载。</p>
    </div>;
  }

  return <div className="connections-panel">
    <div className="connections-header">
      <span className="connections-title">分片连接</span>
      <span className="connections-summary">
        {completedCount}/{totalCount} 已完成 · {activeCount} 活跃{failedCount > 0 ? ` · ${failedCount} 失败` : ""}
      </span>
    </div>
    <div className="connections-overall">
      <div className="connections-overall-bar"><i style={{ width: `${overallPercent}%` }} /></div>
      <span className="connections-overall-text">{formatBytes(totalDownloaded)} / {formatBytes(totalBytes)} ({overallPercent}%)</span>
    </div>
    <div className="connections-list">
      {displayList.map((seg) => {
        const percent = seg.total_bytes > 0 ? Math.min(100, Math.round((seg.downloaded_bytes / seg.total_bytes) * 100)) : 0;
        const meta: string[] = [];
        if (seg.speed > 0) meta.push(`${formatBytes(seg.speed)}/s`);
        if (seg.retry_count > 0) meta.push(`重试 ${seg.retry_count}`);
        return <div key={seg.segment_id} className={`connection-item state-${seg.state}`}>
          <div className="connection-row">
            <span className="connection-index">#{seg.segment_id}</span>
            <span className="connection-state-badge">{CONNECTION_STATE_LABEL[seg.state]}</span>
            <span className="connection-bytes">{formatBytes(seg.downloaded_bytes)} / {formatBytes(seg.total_bytes)}</span>
            <span className="connection-percent">{percent}%</span>
          </div>
          <div className="connection-bar"><i style={{ width: `${percent}%` }} /></div>
          {(meta.length > 0 || (seg.state === "failed" && seg.error)) && (
            <div className="connection-meta">
              {meta.map((m, i) => <span key={i}>{m}</span>)}
              {seg.state === "failed" && seg.error && <span className="connection-error">{seg.error}</span>}
            </div>
          )}
        </div>;
      })}
    </div>
    <div className="connections-footer">
      {live
        ? lastTimestamp
          ? `实时 · 更新于 ${new Date(lastTimestamp).toLocaleTimeString()}`
          : "等待第一次状态推送…"
        : "已停止推送 · 显示为最后一次状态"}
    </div>
  </div>;
}

function DetailsInfoTab({ task, showMore, onToggleMore, notify, action, onTagsChanged }: {
  task: DownloadTask;
  showMore: boolean;
  onToggleMore: () => void;
  notify: (text: string, kind?: "ok" | "error") => void;
  action: (value: string) => Promise<void>;
  onTagsChanged?: () => void;
}) {
  // Task 33: 订阅 locale 变化，详情信息标签页文案同步刷新。
  useLocale();
  const statusText = getStatusText();
  const [waitReason, setWaitReason] = useState<WaitReason | null>(null);
  // Task 16.2: 数字优先级输入框本地状态。用户可自由输入，失焦或回车时提交。
  const [priorityInput, setPriorityInput] = useState(String(task.priority));
  const [prioritySaving, setPrioritySaving] = useState(false);
  const taskId = task.id;
  const taskStatus = task.status;
  const isWaiting = taskStatus === "queued" || taskStatus === "scheduled";

  // 事件驱动刷新：仅在任务处于 Queued/Scheduled 时主动调用 getWaitReason，
  // 并监听 task-updated/task-created/task-removed 事件刷新（其他任务变化可能影响 ahead_count）。
  // 不引入轮询——全部由 Tauri 事件触发。
  useEffect(() => {
    if (!isWaiting) {
      setWaitReason(null);
      return;
    }

    let cancelled = false;
    const unlistens: Array<() => void> = [];

    const fetchReason = () => {
      if (cancelled) return;
      api.getWaitReason(taskId)
        .then(reason => { if (!cancelled) setWaitReason(reason); })
        .catch(() => { if (!cancelled) setWaitReason(null); });
    };

    fetchReason();

    if (isDesktop()) {
      Promise.all([
        listen<DownloadTask>("task-updated", fetchReason),
        listen<DownloadTask>("task-created", fetchReason),
        listen<string>("task-removed", fetchReason),
      ]).then(fns => {
        if (cancelled) {
          fns.forEach(fn => fn());
        } else {
          unlistens.push(...fns);
        }
      });
    }

    return () => {
      cancelled = true;
      unlistens.forEach(fn => fn());
    };
  }, [taskId, taskStatus, isWaiting]);

  // Task 16.2: 任务切换或后端推送新 priority 时同步本地输入框。
  useEffect(() => {
    setPriorityInput(String(task.priority));
  }, [task.id, task.priority]);

  // Task 16.2: 提交数字优先级。校验整数范围后调用 task_update_options；
  // 失败时回退到原值，不抛错（AGENTS.md §7）。
  const commitPriority = async () => {
    if (prioritySaving) return;
    const trimmed = priorityInput.trim();
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) {
      notify(t("toasts.priorityMustBeInteger"), "error");
      setPriorityInput(String(task.priority));
      return;
    }
    const clamped = clampPriority(parsed);
    if (clamped !== parsed) {
      notify(t("toasts.priorityClamped", { min: MIN_PRIORITY, max: MAX_PRIORITY }), "error");
    }
    if (clamped === task.priority) {
      setPriorityInput(String(clamped));
      return;
    }
    setPrioritySaving(true);
    try {
      await api.updateTaskOptions(task.id, { priority: clamped });
      notify(t("toasts.priorityUpdated"));
    } catch (error) {
      notify(String(error), "error");
      setPriorityInput(String(task.priority));
    } finally {
      setPrioritySaving(false);
    }
  };

  const completionLabel = completionActionLabel(task.completion_action);
  const priorityLabel = task.priority < 0 ? t("details.priorityHigh") : task.priority > 0 ? t("details.priorityLow") : t("details.priorityNormal");
  const waitReasonLabel = waitReason ? waitReasonText(waitReason) : null;
  // Task 45：检查 task.headers 是否包含 Cookie/Referer/User-Agent（大小写不敏感）。
  // 后端在下载完成后会清空这些认证头，标记自动消失。
  const hasTempAuth = !!task.headers && Object.keys(task.headers).some((name) => {
    const lower = name.toLowerCase();
    return lower === "cookie" || lower === "referer" || lower === "referrer" || lower === "user-agent";
  });

  // Task 36: 把当前任务保存为任务模板。
  // 自动从 URL 提取域名作为 domain_pattern，套用任务的连接数/限速/请求头/保存目录/完成动作。
  // 用户可在弹出的对话框中修改模板名称；保存后可在设置 → 任务模板中进一步编辑。
  const [saveTplOpen, setSaveTplOpen] = useState(false);
  const [tplDraft, setTplDraft] = useState<TaskTemplate | null>(null);
  const [tplHeadersText, setTplHeadersText] = useState("");

  const openSaveAsTemplate = () => {
    let domain = "";
    try {
      domain = new URL(task.url).hostname.toLowerCase();
    } catch {
      domain = "";
    }
    const headers = task.headers && Object.keys(task.headers).length > 0 ? { ...task.headers } : null;
    setTplDraft({
      id: `tpl-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: domain ? `${domain} ${t("common.custom")}` : t("common.custom"),
      domain_pattern: domain,
      connections: task.connection_count,
      speed_limit: task.per_task_speed_limit || null,
      headers,
      destination: task.destination || null,
      completion_action: task.completion_action === "none" ? null : task.completion_action,
      enabled: true,
      priority: 0,
    });
    setTplHeadersText(
      headers ? Object.entries(headers).map(([k, v]) => `${k}: ${v}`).join("\n") : ""
    );
    setSaveTplOpen(true);
  };

  const saveAsTemplate = async () => {
    if (!tplDraft) return;
    if (!tplDraft.name.trim()) { notify(t("toasts.tagNameEmpty"), "error"); return; }
    if (!tplDraft.domain_pattern.trim()) { notify(t("toasts.tagNameEmpty"), "error"); return; }
    // 解析请求头文本
    let headers: Record<string, string> | null = null;
    const trimmedHeaders = tplHeadersText.trim();
    if (trimmedHeaders) {
      headers = {};
      for (const line of trimmedHeaders.split(/\r?\n/)) {
        const lineTrim = line.trim();
        if (!lineTrim) continue;
        const idx = lineTrim.indexOf(":");
        if (idx <= 0) {
          notify(String(line), "error");
          return;
        }
        headers[lineTrim.slice(0, idx).trim()] = lineTrim.slice(idx + 1).trim();
      }
      if (Object.keys(headers).length === 0) headers = null;
    }
    const toSave: TaskTemplate = {
      ...tplDraft,
      headers,
      destination: tplDraft.destination?.trim() || null,
    };
    try {
      await api.taskTemplateAdd(toSave);
      notify(t("toasts.settingsSaved"));
      setSaveTplOpen(false);
      setTplDraft(null);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  return <>
    <dl>
      <div><dt>{t("details.status")}</dt><dd>{task.status === "downloading" && isMediaTask(task) && task.downloaded_bytes === 0 && task.active_connections === 0 && !task.error ? statusText.parsing : statusText[task.status]}</dd></div>
      <div><dt>{t("details.size")}</dt><dd>{task.total_bytes ? formatBytes(task.total_bytes) : "—"}</dd></div>
      <div><dt>{t("details.speed")}</dt><dd>{task.speed ? `${formatBytes(task.speed)}/s` : "—"}</dd></div>
      <div><dt>{t("details.eta")}</dt><dd>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</dd></div>
      <div><dt>{t("details.sourceDomain")}</dt><dd>{hostOf(task.url)}</dd></div>
      <div><dt>{t("details.saveLocation")}</dt><dd>{task.destination}</dd></div>
      <div>
        <dt>{t("details.priority")}</dt>
        <dd>
          <input
            type="number"
            value={priorityInput}
            min={MIN_PRIORITY}
            max={MAX_PRIORITY}
            step={PRIORITY_STEP}
            disabled={prioritySaving}
            onChange={(e) => setPriorityInput(e.target.value)}
            onBlur={() => void commitPriority()}
            onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
            style={{ width: "88px" }}
            aria-label={t("details.priority")}
          />
          <span style={{ marginLeft: "6px" }} title={t("details.priorityHint")}>{priorityLabel}</span>
          <small style={{ display: "block", marginTop: "2px", fontSize: "10px", opacity: 0.7, whiteSpace: "nowrap" }}>{t("details.priorityRange", { min: MIN_PRIORITY, max: MAX_PRIORITY })}</small>
        </dd>
      </div>
      <div><dt>{t("details.taskSpeedLimit")}</dt><dd>{task.per_task_speed_limit ? `${Math.round(task.per_task_speed_limit / 1024)} KB/s` : t("details.noSpeedLimit")}</dd></div>
      <div><dt>{t("details.completionAction")}</dt><dd>{completionLabel}</dd></div>
      <div><dt>{t("details.downloadSource")}</dt><dd>{task.source}</dd></div>
    </dl>

    {/* Task 45：当 task.headers 含 Cookie/Referer/User-Agent 时展示"包含临时登录态"标记。
        下载完成后端会清空这些认证头，标记自动消失（避免持久化）。 */}
    <div style={{ display: "flex", flexDirection: "column", gap: "6px", marginTop: "-4px" }}>
      {hasTempAuth && (
        <div className="temp-auth-banner" role="status" title={t("details.tempAuthHint")}>
          <ShieldCheck size={13} />
          <span>{t("details.tempAuthBadge")}</span>
        </div>
      )}

      {waitReasonLabel && (
        <div className="wait-reason-banner" role="status">
          <Clock size={13} />
          <span>{waitReasonLabel}</span>
        </div>
      )}

      <button className={`details-more-toggle ${showMore ? "open" : ""}`} onClick={onToggleMore}>
        <ChevronDown size={11} />
        {t("details.moreInfo")}
      </button>

      {showMore && (
        <dl>
          <DetailValue label={t("details.originalUrl")} value={redactedUrl(task.url)} notify={notify} />
          {task.final_url && <DetailValue label={t("details.finalUrl")} value={task.final_url} notify={notify} />}
          {task.response_status && <div><dt>{t("details.httpStatus")}</dt><dd>{task.response_status}</dd></div>}
          {task.content_type && <DetailValue label={t("details.contentType")} value={task.content_type} notify={notify} />}
          {task.accepts_ranges !== undefined && <div><dt>{t("details.acceptsRanges")}</dt><dd>{task.accepts_ranges ? t("details.rangeSupported") : t("details.rangeNotSupported")}</dd></div>}
          {task.etag && <DetailValue label="ETag" value={task.etag} notify={notify} />}
          {task.last_modified && <DetailValue label="Last-Modified" value={task.last_modified} notify={notify} />}
          <div><dt>{t("details.retryCount")}</dt><dd>{task.retry_count} / {task.max_retries}</dd></div>
          {task.checksum_sha256 && <div><dt>{t("details.sha256")}</dt><dd title={task.checksum_sha256}>{task.checksum_sha256.slice(0, 16)}…</dd></div>}
        </dl>
      )}

      <p className="details-security-note" style={{ margin: 0 }}>{t("details.securityNote")}</p>
    </div>

    {task.error && <div className="task-error">{task.error}</div>}

    {task.status === "remote-changed" && (
      <div className="remote-changed-banner" role="alert">
        <AlertCircle size={16} />
        <div className="remote-changed-body">
          <strong>{t("details.remoteChanged")}</strong>
          <p>{t("details.remoteChangedDesc")}</p>
          <div className="remote-changed-actions">
            <button
              className="remote-changed-redownload"
              onClick={() => void action("redownload").then(() => notify(t("toasts.redownloading")))}
              title={t("details.redownloadHint")}
            >
              <RefreshCw size={13} />{t("details.redownloadAction")}
            </button>
            <button
              className="remote-changed-keep"
              onClick={() => void action("cancel")}
              title={t("details.keepOldFileHint")}
            >
              <CirclePause size={13} />{t("details.keepOldFile")}
            </button>
          </div>
        </div>
      </div>
    )}

    {task.status === "paused-by-metered" && (
      <div className="remote-changed-banner" role="status">
        <AlertCircle size={16} />
        <div className="remote-changed-body">
          <strong>{t("details.meteredPaused")}</strong>
          <p>{t("details.meteredPausedDesc")}</p>
          <div className="remote-changed-actions">
            <button
              className="remote-changed-redownload"
              onClick={() => void action("resume").then(() => notify(t("toasts.meteredResumed")))}
              title={t("details.resumeDownloadHint")}
            >
              <Play size={13} />{t("details.resumeDownload")}
            </button>
          </div>
        </div>
      </div>
    )}

    {task.segments.length > 0 && (
      <div className="segment-panel">
        <div className="segment-title">{t("details.segments", { active: task.active_connections, max: task.connection_count, count: task.segments.length })}</div>
        <div className="segment-list">
          {task.segments.map((segment) => {
            const size = segment.end_byte - segment.start_byte + 1;
            const value = size ? Math.min(100, (segment.downloaded_bytes / size) * 100) : 0;
            return (
              <div className={`segment-item ${segment.status === "downloading" && task.status === "downloading" ? "active" : ""}`} key={segment.index}>
                <span>#{segment.index + 1}</span>
                <div><i style={{ width: `${value}%` }} /></div>
                <em>{value.toFixed(0)}%</em>
              </div>
            );
          })}
        </div>
      </div>
    )}

    <div className="details-actions">
      {["downloading", "waiting-network"].includes(task.status) ? (
        <button onClick={() => void action("pause")}><Pause size={13} />{t("details.pauseDownload")}</button>
      ) : (
        !["completed", "cancelled", "remote-changed"].includes(task.status) && <button onClick={() => void action("resume")}><Play size={13} />{t("details.resumeDownload")}</button>
      )}
      <button onClick={() => void api.openFolder(task.id)}><FolderOpen size={13} />{t("details.openDirectory")}</button>
      {task.status === "completed" && (
        <button onClick={async () => {
          try {
            const hash = await api.verify(task.id);
            notify(t("toasts.verifyComplete", { hash: hash.slice(0, 12) }));
          } catch (error) {
            notify(String(error), "error");
          }
        }}><ShieldCheck size={13} />{t("details.verifyFile")}</button>
      )}
      <button onClick={openSaveAsTemplate} title={t("details.saveAsTemplate")}><Bookmark size={13} />{t("details.saveAsTemplate")}</button>
    </div>

    <TaskRetryPolicySection task={task} notify={notify} />
    <TaskProxySection task={task} notify={notify} />
    <TaskTagEditor task={task} notify={notify} onTagsChanged={onTagsChanged} />

    {saveTplOpen && tplDraft && (
      <Modal title="保存为任务模板" onClose={() => setSaveTplOpen(false)} style={{ width: "520px" }}>
        <div className="category-rule-edit-form">
          <p className="settings-note" style={{ margin: "0 0 4px" }}>将当前任务的下载参数保存为模板，下次新建同域名任务时自动套用到未显式设置的字段。</p>
          <div className="template-edit-grid">
            <Field label="模板名称">
              <input value={tplDraft.name} onChange={(e) => setTplDraft({ ...tplDraft, name: e.target.value })} />
            </Field>
            <Field label="域名匹配模式">
              <input value={tplDraft.domain_pattern} onChange={(e) => setTplDraft({ ...tplDraft, domain_pattern: e.target.value })} placeholder="github.com 或 *.github.com" />
            </Field>
            <Field label="连接数（留空表示不覆盖；仅允许 1 / 2 / 4 / 8 / 16 / 32）">
              <Select
                value={tplDraft.connections ?? ""}
                onChange={(val: any) => {
                  setTplDraft({ ...tplDraft, connections: val === "" ? null : +val });
                }}
                options={[
                  { value: "", label: "不覆盖" },
                  { value: 1, label: "1 路" },
                  { value: 2, label: "2 路" },
                  { value: 4, label: "4 路" },
                  { value: 8, label: "8 路" },
                  { value: 16, label: "16 路" },
                  { value: 32, label: "32 路" },
                ]}
                ariaLabel="连接数"
              />
            </Field>
            <Field label="单任务限速（KB/s，0 或留空表示不限速）">
              <input
                type="number"
                min="0"
                value={tplDraft.speed_limit ? Math.round(tplDraft.speed_limit / 1024) : 0}
                onChange={(e) => {
                  const v = +e.target.value;
                  setTplDraft({ ...tplDraft, speed_limit: v > 0 ? v * 1024 : null });
                }}
              />
            </Field>
            <Field className="wide" label="保存目录（留空表示不覆盖）">
              <input
                value={tplDraft.destination ?? ""}
                onChange={(e) => setTplDraft({ ...tplDraft, destination: e.target.value || null })}
                placeholder="例如：D:\\Downloads\\GitHub"
              />
            </Field>
            <Field className="wide" label="请求头（每行一个，格式 Key: Value；留空表示不覆盖）">
              <textarea
                rows={3}
                value={tplHeadersText}
                onChange={(e) => setTplHeadersText(e.target.value)}
                placeholder={"Authorization: Bearer token\nUser-Agent: MaobuFetch"}
                style={{ width: "100%", fontFamily: "monospace" }}
              />
            </Field>
          </div>
          <div className="dialog-actions"><button onClick={() => setSaveTplOpen(false)}>取消</button><button className="primary" onClick={() => void saveAsTemplate()}>保存</button></div>
        </div>
      </Modal>
    )}
  </>;
}

function DetailValue({ label, value, notify }: { label: string; value: string; notify: (text: string, kind?: "ok" | "error") => void }) {
  return <div><dt>{label}</dt><dd className="detail-copy-value" title={value}><span>{value}</span><button onClick={() => void navigator.clipboard.writeText(value).then(() => notify(t("details.copied", { label })))} title={t("details.copyLabel", { label })}><Copy size={11} /></button></dd></div>;
}

const RETRY_PRESETS = {
  standard: {
    connection_timeout_secs: 60,
    task_timeout_secs: null,
    max_retries: 5,
    backoff: "exponential",
    initial_backoff_ms: 1000,
    max_backoff_ms: 60000,
  },
  quick: {
    connection_timeout_secs: 15,
    task_timeout_secs: 300,
    max_retries: 10,
    backoff: "fixed",
    initial_backoff_ms: 1000,
    max_backoff_ms: 1000,
  },
  persistent: {
    connection_timeout_secs: 30,
    task_timeout_secs: null,
    max_retries: 30,
    backoff: "exponential",
    initial_backoff_ms: 2000,
    max_backoff_ms: 300000,
  },
  none: {
    connection_timeout_secs: 30,
    task_timeout_secs: null,
    max_retries: 0,
    backoff: "fixed",
    initial_backoff_ms: 1000,
    max_backoff_ms: 1000,
  }
};

const detectRetryPreset = (policy: RetryPolicy): string => {
  if (policy.max_retries === 0) return "none";
  for (const [key, preset] of Object.entries(RETRY_PRESETS)) {
    if (key === "none") continue;
    const p = preset as any;
    if (
      policy.connection_timeout_secs === p.connection_timeout_secs &&
      policy.task_timeout_secs === p.task_timeout_secs &&
      policy.max_retries === p.max_retries &&
      policy.backoff === p.backoff &&
      policy.initial_backoff_ms === p.initial_backoff_ms &&
      policy.max_backoff_ms === p.max_backoff_ms
    ) {
      return key;
    }
  }
  return "custom";
};

/**
 * 重试策略编辑器（Task 14）。
 * 同时用于设置页（编辑全局默认）和任务详情面板（编辑任务级覆盖）。
 * 提供标准重试、快速重试、顽固重试、不自动重试及自定义预设。
 */
function RetryPolicyEditor({ value, onChange, disabled, compact }: { value: RetryPolicy; onChange: (value: RetryPolicy) => void; disabled?: boolean; compact?: boolean }) {
  const [localPreset, setLocalPreset] = useState<string>(() => detectRetryPreset(value));

  const update = <K extends keyof RetryPolicy>(key: K, val: RetryPolicy[K]) => onChange({ ...value, [key]: val });
  const updateTaskTimeout = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      update("task_timeout_secs", null);
      return;
    }
    const parsed = Number(trimmed);
    if (Number.isFinite(parsed) && parsed > 0) {
      update("task_timeout_secs", Math.floor(parsed));
    }
  };

  const handlePresetChange = (presetKey: string) => {
    setLocalPreset(presetKey);
    if (presetKey !== "custom") {
      const selectedPreset = RETRY_PRESETS[presetKey as keyof typeof RETRY_PRESETS];
      onChange({
        connection_timeout_secs: selectedPreset.connection_timeout_secs,
        task_timeout_secs: selectedPreset.task_timeout_secs,
        max_retries: selectedPreset.max_retries,
        backoff: selectedPreset.backoff as BackoffStrategy,
        initial_backoff_ms: selectedPreset.initial_backoff_ms,
        max_backoff_ms: selectedPreset.max_backoff_ms,
      });
    }
  };

  const taskTimeoutValue = value.task_timeout_secs == null ? "" : String(value.task_timeout_secs);
  const isInputsDisabled = disabled || localPreset !== "custom";
  return (
    <div className="settings-group-content">
      <SettingRow label="重试预设">
        <Select
          value={localPreset}
          disabled={disabled}
          onChange={(val: any) => handlePresetChange(val as string)}
          options={[
            { value: "standard", label: "标准重试 (默认)" },
            { value: "quick", label: "快速重试 (针对不稳定 CDN)" },
            { value: "persistent", label: "顽固重试 (挂机且网络极差)" },
            { value: "none", label: "不自动重试" },
            { value: "custom", label: "自定义配置..." },
          ]}
          ariaLabel="重试预设"
        />
      </SettingRow>
      <div className="retry-policy-advanced-fields">
        <SettingRow label={compact ? "单连接超时(秒)" : "单连接超时（秒）"}><input type="number" min="1" max="600" value={value.connection_timeout_secs} disabled={isInputsDisabled} onChange={(e) => update("connection_timeout_secs", Math.max(1, +e.target.value || 1))} /></SettingRow>
        <SettingRow label={compact ? "任务总超时(秒)" : "任务总超时（秒，留空表示不限制）"}><input type="number" min="0" placeholder={compact ? "不限" : "不限制"} value={taskTimeoutValue} disabled={isInputsDisabled} onChange={(e) => updateTaskTimeout(e.target.value)} /></SettingRow>
        <SettingRow label={compact ? "最大重试次数" : "最大重试次数（每条连接独立计数）"}><input type="number" min="0" max="32" value={value.max_retries} disabled={isInputsDisabled} onChange={(e) => update("max_retries", Math.min(32, Math.max(0, +e.target.value || 0)))} /></SettingRow>
        <SettingRow label="退避策略"><div className="fluent-segmented-control settings-segmented"><button type="button" disabled={isInputsDisabled} className={value.backoff === "fixed" ? "active" : ""} onClick={() => update("backoff", "fixed" as BackoffStrategy)}>固定间隔</button><button type="button" disabled={isInputsDisabled} className={value.backoff === "exponential" ? "active" : ""} onClick={() => update("backoff", "exponential" as BackoffStrategy)}>指数退避</button></div></SettingRow>
        <SettingRow label={compact ? "初始退避(毫秒)" : "初始退避时长（毫秒）"}><input type="number" min="1" value={value.initial_backoff_ms} disabled={isInputsDisabled} onChange={(e) => update("initial_backoff_ms", Math.max(1, +e.target.value || 1))} /></SettingRow>
        <SettingRow label={compact ? "最大退避(毫秒)" : "最大退避时长（毫秒）"}><input type="number" min={value.initial_backoff_ms} value={value.max_backoff_ms} disabled={isInputsDisabled} onChange={(e) => update("max_backoff_ms", Math.max(value.initial_backoff_ms, +e.target.value || value.initial_backoff_ms))} /></SettingRow>
      </div>
    </div>
  );
}

/**
 * 任务级重试策略覆盖编辑区（Task 14）。
 *
 * 显示当前生效的策略来源（全局默认 / 任务覆盖），允许用户在两者之间切换。
 * 切换到"自定义覆盖"时展开编辑器，保存时调用 `api.updateRetryPolicy`。
 * 切换回"使用全局默认"时调用 `api.updateRetryPolicy(id, null)` 清除覆盖。
 *
 * 不影响 v1.1 的 ETag/磁盘空间检查（这些检查不参与重试）。
 */
function TaskRetryPolicySection({ task, notify }: { task: DownloadTask; notify: (text: string, kind?: "ok" | "error") => void }) {
  const hasOverride = task.retry_policy_override != null;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<RetryPolicy | null>(task.retry_policy_override ?? null);
  const [saving, setSaving] = useState(false);

  // 任务切换或外部更新时同步本地草稿。
  useEffect(() => {
    setDraft(task.retry_policy_override ?? null);
    setEditing(false);
  }, [task.id, task.retry_policy_override]);

  const backoffLabel = (policy: RetryPolicy | null | undefined) => {
    if (!policy) return "全局默认";
    return policy.backoff === "exponential" ? "指数退避" : "固定间隔";
  };
  const summary = (policy: RetryPolicy | null | undefined) => {
    if (!policy) return "使用全局默认策略";
    const timeout = policy.task_timeout_secs == null ? "无总超时" : `总超时 ${policy.task_timeout_secs}s`;
    return `${policy.connection_timeout_secs}s 连接超时 · ${timeout} · ${policy.max_retries} 次重试 · ${backoffLabel(policy)}`;
  };

  const startEdit = () => {
    // 进入编辑时若当前没有覆盖，则基于全局默认复制一份作为草稿起点。
    setDraft(task.retry_policy_override ?? { connection_timeout_secs: 60, task_timeout_secs: null, max_retries: 5, backoff: "exponential", initial_backoff_ms: 1000, max_backoff_ms: 60000 });
    setEditing(true);
  };

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    try {
      await api.updateRetryPolicy(task.id, draft);
      notify("任务重试策略已保存");
      setEditing(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const clearOverride = async () => {
    setSaving(true);
    try {
      await api.updateRetryPolicy(task.id, null);
      notify("已恢复使用全局默认重试策略");
      setEditing(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="task-retry-policy-section">
      <div className="task-retry-policy-header">
        <strong>重试策略</strong>
        <span className="task-retry-policy-summary">
          {editing ? "✏️ 正在编辑自定义重试策略" : summary(task.retry_policy_override)}
        </span>
      </div>
      {!editing && (
        <div className="task-retry-policy-actions">
          <button onClick={startEdit} disabled={saving}>自定义覆盖</button>
          {hasOverride && <button onClick={() => void clearOverride()} disabled={saving}>恢复全局默认</button>}
        </div>
      )}
      {editing && draft && (
        <div className="task-retry-policy-editor retry-policy-grid">
          <RetryPolicyEditor value={draft} onChange={setDraft} disabled={saving} compact />
          <div className="task-retry-policy-actions">
            <button className="primary" onClick={() => void save()} disabled={saving}>{saving ? "保存中…" : "保存覆盖"}</button>
            <button onClick={() => { setEditing(false); setDraft(task.retry_policy_override ?? null); }} disabled={saving}>取消</button>
            {hasOverride && <button onClick={() => void clearOverride()} disabled={saving}>清除覆盖</button>}
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * Task 31：任务级代理覆盖编辑区。
 *
 * 显示当前生效的代理来源（全局默认 / 任务覆盖 / 显式禁用），
 * 允许用户在三种状态之间切换：
 * - 使用全局：清除 `proxy_override`，回退到 `AppSettings.proxy_mode`。
 * - 显式禁用：设置 `proxy_override = ""`，即使全局是 manual 也不走代理。
 * - 自定义 URL：设置 `proxy_override = url`，可选附加认证。
 *
 * 保存时调用 `api.updateTaskProxy`；密码非空时由后端 DPAPI 加密后落库。
 * `task.proxy_auth.password` 在内存中是 DPAPI 密文，前端不展示原始值。
 */
function TaskProxySection({ task, notify }: { task: DownloadTask; notify: (text: string, kind?: "ok" | "error") => void }) {
  const [editing, setEditing] = useState(false);
  // 草稿状态：mode ∈ "global" | "disable" | "custom"；customUrl、username、password 仅在 custom 模式下生效。
  const [mode, setMode] = useState<"global" | "disable" | "custom">(
    task.proxy_override == null ? "global" : task.proxy_override === "" ? "disable" : "custom"
  );
  const [customUrl, setCustomUrl] = useState(task.proxy_override ?? "");
  const [username, setUsername] = useState(task.proxy_auth?.username ?? "");
  // 密码草稿：进入编辑时清空，避免展示 DPAPI 密文；用户未填写时保持空，保存为 null。
  const [password, setPassword] = useState("");
  const [saving, setSaving] = useState(false);

  // 任务切换或外部更新时同步本地草稿。
  useEffect(() => {
    setMode(task.proxy_override == null ? "global" : task.proxy_override === "" ? "disable" : "custom");
    setCustomUrl(task.proxy_override ?? "");
    setUsername(task.proxy_auth?.username ?? "");
    setPassword("");
    setEditing(false);
  }, [task.id, task.proxy_override, task.proxy_auth]);

  const summary = () => {
    if (task.proxy_override == null) return "使用全局代理设置";
    if (task.proxy_override === "") return "不使用代理";
    return `手动代理：${task.proxy_override}`;
  };

  const startEdit = () => {
    // 进入编辑时基于当前状态初始化草稿；密码始终清空，用户未填则不更新密码。
    setMode(task.proxy_override == null ? "global" : task.proxy_override === "" ? "disable" : "custom");
    setCustomUrl(task.proxy_override && task.proxy_override !== "" ? task.proxy_override : "");
    setUsername(task.proxy_auth?.username ?? "");
    setPassword("");
    setEditing(true);
  };

  const save = async () => {
    setSaving(true);
    try {
      let override: string | null;
      if (mode === "global") {
        override = null;
      } else if (mode === "disable") {
        override = "";
      } else {
        const trimmed = customUrl.trim();
        if (!trimmed) {
          notify("自定义代理地址不能为空", "error");
          setSaving(false);
          return;
        }
        override = trimmed;
      }
      // 认证：仅 custom 模式且用户名非空时附加；密码为空时不修改（保留旧密码），
      // 但当前后端无"仅更新用户名"的语义——密码为空时整体视为无认证。
      // 这是简化实现：用户每次保存都需重新输入密码。
      const auth: ProxyAuth | null = mode === "custom" && username.trim()
        ? { username: username.trim(), password }
        : null;
      await api.updateTaskProxy(task.id, override, auth);
      notify("任务代理设置已保存");
      setEditing(false);
      setPassword("");
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const clearOverride = async () => {
    setSaving(true);
    try {
      await api.updateTaskProxy(task.id, null, null);
      notify("已恢复使用全局代理设置");
      setEditing(false);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const hasOverride = task.proxy_override != null;

  return (
    <div className="task-retry-policy-section">
      <div className="task-retry-policy-header">
        <strong>代理覆盖</strong>
        <span className="task-retry-policy-summary">
          {editing ? "✏️ 正在编辑代理覆盖设置" : summary()}
        </span>
      </div>
      {!editing && (
        <div className="task-retry-policy-actions">
          <button onClick={startEdit} disabled={saving}>编辑代理</button>
          {hasOverride && <button onClick={() => void clearOverride()} disabled={saving}>恢复全局默认</button>}
        </div>
      )}
      {editing && (
        <div className="task-retry-policy-editor">
          <div className="settings-group-content">
            <SettingRow label="代理模式">
              <div className="fluent-segmented-control settings-segmented">
                <button type="button" disabled={saving} className={mode === "global" ? "active" : ""} onClick={() => setMode("global")}>使用全局</button>
                <button type="button" disabled={saving} className={mode === "disable" ? "active" : ""} onClick={() => setMode("disable")}>不使用代理</button>
                <button type="button" disabled={saving} className={mode === "custom" ? "active" : ""} onClick={() => setMode("custom")}>手动代理</button>
              </div>
            </SettingRow>
            {mode === "custom" && <>
              <SettingRow label="代理地址"><input value={customUrl} onChange={(e) => setCustomUrl(e.target.value)} placeholder="http://host:port 或 socks5://host:port" disabled={saving} /></SettingRow>
              <SettingRow label="用户名"><input value={username} onChange={(e) => setUsername(e.target.value)} placeholder="匿名代理可留空" disabled={saving} /></SettingRow>
              <SettingRow label="密码"><input type="password" value={password} onChange={(e) => setPassword(e.target.value)} placeholder="留空表示无认证" disabled={saving} /></SettingRow>
              <SettingRow label="测试连通性">
                <ProxyTestButton
                  proxyUrl={customUrl}
                  auth={username || password ? { username, password } : null}
                  notify={notify}
                  disabled={saving}
                />
              </SettingRow>
            </>}
          </div>
          <div className="task-retry-policy-actions">
            <button className="primary" onClick={() => void save()} disabled={saving}>{saving ? "保存中…" : "保存"}</button>
            <button onClick={() => { setEditing(false); setPassword(""); }} disabled={saving}>取消</button>
            {hasOverride && <button onClick={() => void clearOverride()} disabled={saving}>清除覆盖</button>}
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * Task 25: 任务详情面板中的标签编辑器。
 *
 * - 加载该任务当前关联的标签 id 集合
 * - 提供下拉多选标签 + "新建标签"快捷入口（输入名称+颜色后立即创建并附加）
 * - 保存时调用 task_tags_set 替换全部关联
 * - 不显示 chip 颜色选择器（颜色仅在设置页统一管理），但 chip 显示颜色
 */
function TaskTagEditor({ task, notify, onTagsChanged }: { task: DownloadTask; notify: (text: string, kind?: "ok" | "error") => void; onTagsChanged?: () => void }) {
  const [allTags, setAllTags] = useState<Tag[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState("#3B82F6");
  const [showCreateRow, setShowCreateRow] = useState(false);
  const taskId = task.id;

  // 初始加载：拉取全部标签 + 该任务的关联
  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setLoading(true);
      try {
        const [all, mine] = await Promise.all([api.tagList(), api.taskTagsGet(taskId)]);
        if (cancelled) return;
        setAllTags(all);
        setSelectedIds(new Set(mine.map((t) => t.id)));
      } catch (error) {
        if (!cancelled) notify(String(error), "error");
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    void load();
    return () => { cancelled = true; };
  }, [taskId, notify]);

  const toggle = (id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const save = async () => {
    setSaving(true);
    try {
      await api.taskTagsSet(taskId, [...selectedIds]);
      notify("标签已更新");
      onTagsChanged?.();
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const createAndAttach = async () => {
    const name = newName.trim();
    if (!name) {
      notify("标签名称不能为空", "error");
      return;
    }
    if (!/^#[0-9A-Fa-f]{6}$/.test(newColor)) {
      notify("颜色格式必须为 #RRGGBB", "error");
      return;
    }
    setSaving(true);
    try {
      const created = await api.tagAdd({ id: newTagId(), name, color: newColor });
      setAllTags((current) => [...current, created].sort((a, b) => a.name.localeCompare(b.name)));
      setSelectedIds((current) => new Set([...current, created.id]));
      await api.taskTagsSet(taskId, [...selectedIds, created.id]);
      setNewName("");
      setShowCreateRow(false);
      notify("已创建并附加标签");
      onTagsChanged?.();
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="details-section">
      <div className="details-section-title">
        <TagIcon size={13} />
        <span>标签</span>
      </div>
      {loading ? (
        <p className="muted">加载中…</p>
      ) : allTags.length === 0 ? (
        <p className="muted">暂无标签</p>
      ) : (
        <div className="tag-editor-grid" role="group" aria-label="选择标签">
          {allTags.map((tag) => {
            const checked = selectedIds.has(tag.id);
            return (
              <label key={tag.id} className={`tag-editor-chip${checked ? " checked" : ""}`}>
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={() => toggle(tag.id)}
                  aria-label={tag.name}
                />
                <span className="tag-editor-swatch" style={{ background: tag.color }} aria-hidden="true" />
                <span className="tag-editor-name">{tag.name}</span>
              </label>
            );
          })}
        </div>
      )}
      <div className="tag-editor-actions">
        <button
          className="secondary"
          onClick={() => setShowCreateRow((v) => !v)}
          disabled={saving}
        >
          <Plus size={12} />新建标签
        </button>
        <button className="primary" onClick={() => void save()} disabled={saving || loading}>
          {saving ? "保存中…" : "保存"}
        </button>
      </div>
      {showCreateRow && (
        <div className="tag-editor-create-row">
          <input
            type="text"
            placeholder="标签名称"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            maxLength={20}
            aria-label="新标签名称"
          />
          <div className="tag-editor-create-tools">
            <input
              type="color"
              value={newColor}
              onChange={(e) => setNewColor(e.target.value.toUpperCase())}
              aria-label="新标签颜色"
              title="标签颜色"
            />
            <span className="tag-color-hex">{newColor}</span>
            <button className="primary" onClick={() => void createAndAttach()} disabled={saving || !newName.trim()}>
              创建并附加
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * Task 25: 高级筛选面板。多维度筛选：状态/域名/日期/大小/标签/来源。
 * 受控组件，由父组件持有 advancedFilter 状态。
 */
function AdvancedFilterPanel({ value, onChange, tags, quickViews, onApplyQuickView, onSaveQuickView, onDeleteQuickView, onClear }: {
  value: AdvancedFilter;
  onChange: (next: AdvancedFilter) => void;
  tags: Tag[];
  quickViews: QuickView[];
  onApplyQuickView: (view: QuickView) => void;
  onSaveQuickView: (name: string) => void;
  onDeleteQuickView: (id: string) => void;
  onClear: () => void;
}) {
  const [saveName, setSaveName] = useState("");
  const [showSaveInput, setShowSaveInput] = useState(false);
  const allStatuses: TaskStatus[] = ["queued", "downloading", "paused", "completed", "failed", "cancelled", "scheduled", "verifying", "waiting-network", "remote-changed", "interrupted", "paused-by-low-disk", "paused-by-metered"];

  const sourceKeys: Record<string, string> = {
    manual: "advancedFilter.sourceManual",
    extension: "advancedFilter.sourceExtension",
    "deep-link": "advancedFilter.sourceDeepLink",
    desktop: "advancedFilter.sourceDesktop",
    clipboard: "advancedFilter.sourceClipboard",
  };

  const toggleStatus = (status: TaskStatus) => {
    onChange({
      ...value,
      statuses: value.statuses.includes(status)
        ? value.statuses.filter((s) => s !== status)
        : [...value.statuses, status],
    });
  };
  const toggleTag = (id: string) => {
    onChange({
      ...value,
      tagIds: value.tagIds.includes(id)
        ? value.tagIds.filter((t) => t !== id)
        : [...value.tagIds, id],
    });
  };
  const toggleSource = (src: string) => {
    onChange({
      ...value,
      sources: value.sources.includes(src)
        ? value.sources.filter((s) => s !== src)
        : [...value.sources, src],
    });
  };
  const isEmpty = isAdvancedFilterEmpty(value);

  return (
    <div className="advanced-filter-panel" role="region" aria-label={t("advancedFilter.title")}>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">{t("advancedFilter.status")}</span>
        <div className="advanced-filter-chips">
          {allStatuses.map((status) => (
            <button
              key={status}
              className={`filter-chip${value.statuses.includes(status) ? " active" : ""}`}
              onClick={() => toggleStatus(status)}
              type="button"
              aria-pressed={value.statuses.includes(status)}
            >
              {t("statusFilter." + status)}
            </button>
          ))}
        </div>
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">{t("advancedFilter.source")}</span>
        <div className="advanced-filter-chips">
          {Object.keys(sourceKeys).map((src) => (
            <button
              key={src}
              className={`filter-chip${value.sources.includes(src) ? " active" : ""}`}
              onClick={() => toggleSource(src)}
              type="button"
              aria-pressed={value.sources.includes(src)}
            >
              {t(sourceKeys[src])}
            </button>
          ))}
        </div>
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">{t("advancedFilter.domain")}</span>
        <input
          type="text"
          placeholder={t("advancedFilter.domainPlaceholder")}
          value={value.domain}
          onChange={(e) => onChange({ ...value, domain: e.target.value })}
          aria-label={t("advancedFilter.domain")}
        />
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">{t("advancedFilter.addedDate")}</span>
        <input
          type="date"
          aria-label={t("advancedFilter.startDate")}
          value={value.dateFrom ? new Date(value.dateFrom).toISOString().slice(0, 10) : ""}
          onChange={(e) => {
            const v = e.target.value;
            onChange({ ...value, dateFrom: v ? new Date(v + "T00:00:00").getTime() : null });
          }}
        />
        <span>{t("historyFilter.to")}</span>
        <input
          type="date"
          aria-label={t("advancedFilter.endDate")}
          value={value.dateTo ? new Date(value.dateTo).toISOString().slice(0, 10) : ""}
          onChange={(e) => {
            const v = e.target.value;
            onChange({ ...value, dateTo: v ? new Date(v + "T23:59:59.999").getTime() : null });
          }}
        />
      </div>
      <div className="advanced-filter-row">
        <span className="advanced-filter-label">{t("advancedFilter.sizeRange")}</span>
        <input
          type="number"
          min={0}
          placeholder={t("advancedFilter.sizeMinPlaceholder")}
          value={value.sizeMin != null ? (value.sizeMin / (1024 * 1024)).toString() : ""}
          onChange={(e) => {
            const v = e.target.value ? Number(e.target.value) * 1024 * 1024 : null;
            onChange({ ...value, sizeMin: v != null && Number.isFinite(v) ? Math.max(0, v) : null });
          }}
          aria-label={t("advancedFilter.sizeMinPlaceholder")}
        />
        <span>{t("historyFilter.to")}</span>
        <input
          type="number"
          min={0}
          placeholder={t("advancedFilter.sizeMaxPlaceholder")}
          value={value.sizeMax != null ? (value.sizeMax / (1024 * 1024)).toString() : ""}
          onChange={(e) => {
            const v = e.target.value ? Number(e.target.value) * 1024 * 1024 : null;
            onChange({ ...value, sizeMax: v != null && Number.isFinite(v) ? Math.max(0, v) : null });
          }}
          aria-label={t("advancedFilter.sizeMaxPlaceholder")}
        />
      </div>
      {tags.length > 0 && (
        <div className="advanced-filter-row">
          <span className="advanced-filter-label">{t("advancedFilter.tags")}</span>
          <div className="advanced-filter-chips">
            {tags.map((tag) => (
              <button
                key={tag.id}
                className={`filter-chip with-color${value.tagIds.includes(tag.id) ? " active" : ""}`}
                style={value.tagIds.includes(tag.id) ? { background: tag.color, borderColor: tag.color } : undefined}
                onClick={() => toggleTag(tag.id)}
                type="button"
                aria-pressed={value.tagIds.includes(tag.id)}
                title={tag.name}
              >
                {tag.name}
              </button>
            ))}
          </div>
        </div>
      )}
      <div className="advanced-filter-actions">
        <button onClick={onClear} disabled={isEmpty} type="button">{t("advancedFilter.clearFilter")}</button>
        {showSaveInput ? (
          <>
            <input
              type="text"
              placeholder={t("advancedFilter.quickViewName")}
              value={saveName}
              onChange={(e) => setSaveName(e.target.value)}
              maxLength={20}
              aria-label={t("advancedFilter.quickViewName")}
            />
            <button
              onClick={() => {
                if (saveName.trim()) {
                  onSaveQuickView(saveName.trim());
                  setSaveName("");
                  setShowSaveInput(false);
                }
              }}
              disabled={!saveName.trim() || isEmpty}
              type="button"
            >
              {t("common.save")}
            </button>
            <button onClick={() => { setShowSaveInput(false); setSaveName(""); }} type="button">{t("common.cancel")}</button>
          </>
        ) : (
          <button onClick={() => setShowSaveInput(true)} disabled={isEmpty} type="button">
            <Save size={12} />{t("advancedFilter.saveAsQuickView")}
          </button>
        )}
      </div>
      {quickViews.length > 0 && (
        <div className="advanced-filter-row quick-views">
          <span className="advanced-filter-label">{t("advancedFilter.quickViews")}</span>
          <div className="advanced-filter-chips">
            {quickViews.map((qv) => (
              <span key={qv.id} className="quick-view-chip">
                <button
                  className="filter-chip"
                  onClick={() => onApplyQuickView(qv)}
                  type="button"
                  title={t("advancedFilter.applyQuickView", { name: qv.name })}
                >
                  {qv.name}
                </button>
                <button
                  className="quick-view-delete"
                  onClick={() => onDeleteQuickView(qv.id)}
                  type="button"
                  title={t("advancedFilter.deleteQuickView")}
                  aria-label={t("advancedFilter.deleteQuickView")}
                >
                  <X size={10} />
                </button>
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function extractFileNameFromUrl(url: string): string {
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

/**
 * 解析多行 URL 输入（Task 19）。
 *
 * - 按换行符拆分，去除每行首尾空白
 * - 过滤空行
 * - 过滤非 http/https 行（如纯文本注释、空行）—— 不计入 `lines`，但计入 `skippedCount`
 * - 去重：同一 URL 只保留首次出现，重复条目计入 `duplicateCount`
 *
 * 返回的 `lines` 顺序为首次出现顺序，与原 textarea 顺序一致。
 * 单行场景下 `lines.length === 1`，与旧逻辑行为兼容。
 */
function parseMultilineUrls(input: string): { lines: string[]; skippedCount: number; duplicateCount: number } {
  const rawLines = input.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  const seen = new Set<string>();
  const lines: string[] = [];
  let skippedCount = 0;
  let duplicateCount = 0;
  const urlRegex = /https?:\/\/[^\s<>"']+/i;
  for (const line of rawLines) {
    const match = line.match(urlRegex);
    if (!match) {
      skippedCount += 1;
      continue;
    }
    const extracted = match[0];
    if (seen.has(extracted)) {
      duplicateCount += 1;
      continue;
    }
    seen.add(extracted);
    lines.push(extracted);
  }
  return { lines, skippedCount, duplicateCount };
}

// Task 42：图集场景下的图片网格选择器。
// 接收 formats 列表（已由后端填充 image_url），渲染为 CSS Grid，支持全选/反选/单选。
// 纯 CSS Grid 实现，不引入 UI 组件库（AGENTS.md §8）。
// checkbox + 边框双重指示选中状态，键盘可聚焦（AGENTS.md §4 多选可识别）。
function GalleryPicker({ formats, thumbnail, selectedIds, onChange }: {
  formats: MediaFormat[];
  thumbnail?: string;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
}) {
  const imageItems = useMemo(() => formats.filter((item) => item.image_url), [formats]);
  const allSelected = imageItems.length > 0 && selectedIds.size === imageItems.length;

  const toggle = (id: string) => {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChange(next);
  };
  const selectAll = () => onChange(new Set(imageItems.map((item) => item.id)));
  const invert = () => {
    const next = new Set<string>();
    for (const item of imageItems) {
      if (!selectedIds.has(item.id)) next.add(item.id);
    }
    onChange(next);
  };

  // 拖拽多选（与 EpisodePicker 一致）：按下鼠标左键拖过条目可批量选择/取消
  const isDraggingRef = useRef(false);
  const dragTargetStateRef = useRef(true);
  useEffect(() => {
    const handleMouseUp = () => { isDraggingRef.current = false; };
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
            已选 <span style={{ color: "var(--accent, #0078d4)" }}>{selectedIds.size}</span>/{imageItems.length} 张
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
                background: selected ? "var(--accent-bg-subtle, rgba(0, 120, 212, 0.1))" : "var(--item-bg, rgba(255, 255, 255, 0.02))",
                border: selected ? "1px solid var(--accent, #0078d4)" : "1px solid var(--border-color, rgba(255, 255, 255, 0.05))",
                cursor: "pointer",
                transition: "all 0.15s ease",
                userSelect: "none",
              }}
            >
              <div style={{ color: selected ? "var(--accent, #0078d4)" : "var(--text-tertiary)", display: "flex", alignItems: "center" }}>
                {selected ? <CheckSquare size={14} /> : <Square size={14} />}
              </div>
              <span
                style={{
                  padding: "1px 6px",
                  borderRadius: "3px",
                  fontSize: "10px",
                  fontWeight: 600,
                  background: selected ? "var(--accent, #0078d4)" : "rgba(255, 255, 255, 0.1)",
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
                    // 加载失败时隐藏缩略图，避免破图（AGENTS.md §4 错误状态需明确反馈）
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
                  color: selected ? "var(--text-primary)" : "var(--text-secondary)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {item.label || `图片 ${index + 1}`}
              </span>
              <span style={{ fontSize: "11px", color: "var(--text-tertiary)", whiteSpace: "nowrap" }}>
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

function applyFilenameCleanup(fileName: string, rules: FilenameCleanupRule[]): string {
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

function isDownloadableUrlForDialog(url: string): boolean {
  try {
    const trimmed = url.trim();
    return /^https?:\/\/[^\s]+$/i.test(trimmed);
  } catch {
    return false;
  }
}

function NewTaskDialog({ settings, allTasks, onClose, onCreated, defaultUrl, onLocateTask, notify }: { settings: AppSettings; allTasks?: DownloadTask[]; onClose: () => void; onCreated: (tasks: DownloadTask | DownloadTask[]) => void; defaultUrl?: string; onLocateTask?: (taskId: string) => void; notify?: (text: string, kind?: "ok" | "error") => void }) {
  const [urls, setUrls] = useState(defaultUrl || ""); const [destination, setDestination] = useState(settings.download_dir);
  const [fileName, setFileName] = useState(() => {
    if (defaultUrl) {
      const initLines = defaultUrl.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
      if (initLines.length === 1) {
        return extractFileNameFromUrl(initLines[0]);
      }
    }
    return "";
  }); const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false); const [error, setError] = useState<string>();
  const [schedule, setSchedule] = useState(""); const [policy, setPolicy] = useState<CollisionPolicy>(settings.default_collision_policy);
  const [priority, setPriority] = useState(0);
  const [completionAction, setCompletionAction] = useState<CompletionAction>(settings.default_completion_action);
  const [referer, setReferer] = useState(""); const [cookie, setCookie] = useState(""); const [authorization, setAuthorization] = useState("");
  const [checksum, setChecksum] = useState(""); const [limit, setLimit] = useState(0);
  const [connections, setConnections] = useState(settings.connections_per_download);
  const [media, setMedia] = useState<MediaProbeResult>(); const [format, setFormat] = useState("");
  // Task 42：图集场景下用户选中的图片 format id 集合。默认全选，用户可取消勾选。
  const [selectedImageIds, setSelectedImageIds] = useState<Set<string>>(new Set());
  // Task 47：合集/多 P 场景下用户选中的分 P 序号集合与画质偏好。
  const [selectedEpisodeIndices, setSelectedEpisodeIndices] = useState<Set<number>>(new Set());
  const [collectionQualityPreference, setCollectionQualityPreference] = useState<string>("best");
  const [toolStatus, setToolStatus] = useState<ToolStatus>();
  // 预检状态（SubTask 9.1）
  const [precheck, setPrecheck] = useState<PrecheckResult>();
  const [precheckLoading, setPrecheckLoading] = useState(false);
  const [precheckError, setPrecheckError] = useState<string>();
  const [ignoreUrlConflict, setIgnoreUrlConflict] = useState(false);
  // 重复检测状态（SubTask 10.5）
  const [duplicateResult, setDuplicateResult] = useState<DuplicateCheckResult>();
  // 下载预设（Task 12）
  const [presets, setPresets] = useState<DownloadPreset[]>([]);
  const [selectedPresetId, setSelectedPresetId] = useState<string>("");
  // Task 36: 任务模板匹配结果。当 URL 命中某模板时，展示提示。
  const [templateMatch, setTemplateMatch] = useState<TaskTemplateTestResult>();
  // Task 37: 媒体平台识别结果。当 URL 命中抖音/TikTok/Twitter/YouTube/B站/微博时，
  // 在 URL 输入框下方展示"检测到：{平台}"提示。
  const [detectedPlatform, setDetectedPlatform] = useState<MediaPlatform | null>(null);
  // Task 44: 平台兼容性记录。检测到平台后查询对应支持级别，
  // 在 URL 输入框下方展示徽章（已验证/实验性/不支持）。
  // `null` 表示尚未查询或平台为 unknown；`unsupported` 时禁用下载按钮。
  const [platformCompat, setPlatformCompat] = useState<PlatformCompatibility | null>(null);
  // Task 46: 媒体凭证匹配结果。当 URL 域名已保存凭证时，展示提示。
  const [matchedCredentialDomain, setMatchedCredentialDomain] = useState<string | null>(null);
  // Task 41: URL 规范化预览。当用户粘贴分享文本或短链时，后端返回展开后的最终 URL，
  // 在输入框下方展示"原文本 → 规范化 URL"提示。仅当结果与输入不同时显示，避免噪音。
  const [normalizedUrlPreview, setNormalizedUrlPreview] = useState<string | null>(null);
  const fileNameInputRef = useRef<HTMLInputElement | null>(null);
  const userEditedFileName = useRef(false);
  const userEditedConnections = useRef(false);
  const userEditedDestination = useRef(false);
  const precheckSeqRef = useRef(0);
  useEffect(() => { let unlisten: (() => void) | undefined; void api.toolStatus().then(setToolStatus); void api.subscribeMediaTools(setToolStatus).then((value) => { unlisten = value; }); return () => unlisten?.(); }, []);
  // Task 12: 启动时加载预设列表（内置 + 自定义）
  useEffect(() => { void api.presetList().then((list) => setPresets(list ?? [])).catch(() => setPresets([])); }, []);
  // Task 19: 加载 URL 历史（最近 20 条），用于输入框下拉提示
  const [urlHistory, setUrlHistory] = useState<UrlHistoryEntry[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const reloadHistory = () => { void api.urlHistoryList().then(setUrlHistory).catch(() => setUrlHistory([])); };
  useEffect(() => { reloadHistory(); }, []);
  // Task 20: 加载文件名清理规则，用于新建任务对话框的文件名自动清理。
  const [cleanupRules, setCleanupRules] = useState<FilenameCleanupRule[]>([]);
  useEffect(() => { void api.filenameCleanupRuleList().then(setCleanupRules).catch(() => setCleanupRules([])); }, []);
  // Task 20: 文件名或规则变化时，同步调用前端清理算法更新输入框。
  // 仅在用户未手动编辑时自动更新，不加任何延迟，彻底消除视觉闪烁。
  useEffect(() => {
    if (!fileName || cleanupRules.length === 0 || userEditedFileName.current) {
      return;
    }
    const cleaned = applyFilenameCleanup(fileName, cleanupRules);
    if (cleaned !== fileName) {
      setFileName(cleaned);
    }
  }, [fileName, cleanupRules]);
  // Task 19: 解析多行 URL，过滤空行 / 非 http(s) 行，去重（保留首次出现的顺序）。
  // `lines` 用于后续预检 / 批量创建；`skippedCount` 用于 UI 反馈被忽略的行数。
  const { lines, skippedCount, duplicateCount } = parseMultilineUrls(urls);
  // Task 11: 自动分类与保存规则支持。当 URL/文件名/Content-Type 变化时自动匹配规则预填目录（仅在用户未手动选择目录时）。
  useEffect(() => {
    const firstUrl = lines[0];
    if (!firstUrl || userEditedDestination.current) return;
    let active = true;
    void api.categoryRuleApply(firstUrl, fileName || "", precheck?.content_type).then((matched) => {
      if (active && matched && matched.trim() && !userEditedDestination.current) {
        setDestination(matched);
      }
    });
    return () => { active = false; };
  }, [lines, fileName, precheck?.content_type]);
  // Task 36: URL 变化时 debounce 300ms 调用 task_template_test，命中则展示提示。
  useEffect(() => {
    const firstUrl = lines[0];
    if (!firstUrl || lines.length !== 1 || !isDownloadableUrlForDialog(firstUrl)) {
      setTemplateMatch(undefined);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const result = await api.taskTemplateTest(firstUrl);
        if (!cancelled) setTemplateMatch(result);
      } catch {
        if (!cancelled) setTemplateMatch(undefined);
      }
    }, 300);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [lines]);

  // Task 37: URL 变化时 debounce 300ms 调用 media_detect_platform，命中非 unknown
  // 平台时展示"检测到：{平台}"提示。失败时静默忽略，不阻断主流程。
  useEffect(() => {
    const firstUrl = lines[0];
    if (!firstUrl || lines.length !== 1 || !isDownloadableUrlForDialog(firstUrl)) {
      setDetectedPlatform(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const platform = await api.mediaDetectPlatform(firstUrl);
        if (!cancelled) setDetectedPlatform(platform);
      } catch {
        if (!cancelled) setDetectedPlatform(null);
      }
    }, 300);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [lines]);

  // Task 44: detectedPlatform 变化时查询对应平台的兼容性记录。
  // null/unknown 时清空徽章；其它平台调用 platform_compatibility_get
  // 获取支持级别。失败时静默忽略，不阻断主流程（按"实验性"展示更安全）。
  useEffect(() => {
    if (!detectedPlatform || detectedPlatform === "unknown") {
      setPlatformCompat(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const compat = await api.platformCompatibilityGet(detectedPlatform);
        if (!cancelled) setPlatformCompat(compat);
      } catch {
        if (!cancelled) setPlatformCompat(null);
      }
    })();
    return () => { cancelled = true; };
  }, [detectedPlatform]);
  // Task 46: URL 变化时 debounce 300ms 检查域名是否已保存凭证，命中则展示提示。
  // 仅展示"已保存的 {domain} 凭证已应用"提示，不暴露 Cookie 内容。
  useEffect(() => {
    const firstUrl = lines[0];
    if (!firstUrl || lines.length !== 1 || !isDownloadableUrlForDialog(firstUrl)) {
      setMatchedCredentialDomain(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const domain = extractDomainForHint(firstUrl);
        if (!domain) {
          if (!cancelled) setMatchedCredentialDomain(null);
          return;
        }
        const credential = await api.mediaCredentialGet(domain);
        if (!cancelled) setMatchedCredentialDomain(credential ? domain : null);
      } catch {
        // 解密失败或查询失败时不展示提示，不阻塞新建任务流程
        if (!cancelled) setMatchedCredentialDomain(null);
      }
    }, 300);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [lines]);

  // Task 41: URL 变化时 debounce 400ms 调用 media_normalize_url，展示规范化预览。
  // 后端会从分享文本提取 URL、跟随短链重定向、剥离跟踪参数。
  // 仅在返回值与输入不同时显示预览（如分享文本被解析、短链被展开或跟踪参数被剥离）。
  // 失败时静默忽略，不阻断主流程（用户仍可直接提交原始输入）。
  useEffect(() => {
    const trimmed = urls.trim();
    if (!trimmed) {
      setNormalizedUrlPreview(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const normalized = await api.mediaNormalizeUrl(trimmed);
        if (cancelled) return;
        setNormalizedUrlPreview(normalized);
      } catch {
        // 解析失败不展示预览，不阻断主流程
        if (!cancelled) setNormalizedUrlPreview(null);
      }
    }, 400);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [urls]);

  // URL 输入后 debounce 400ms 自动触发预检（SubTask 9.1）+ 重复检测（SubTask 10.5）。
  useEffect(() => {
    const firstUrl = lines[0];
    setIgnoreUrlConflict(false);
    if (!firstUrl || lines.length !== 1 || !isDownloadableUrlForDialog(firstUrl)) {
      setPrecheck(undefined);
      setPrecheckError(undefined);
      setPrecheckLoading(false);
      setDuplicateResult(undefined);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      if (cancelled) return;
      const seq = ++precheckSeqRef.current;
      setPrecheckLoading(true);
      setPrecheckError(undefined);
      setDuplicateResult(undefined);

      const reqHeaders: Record<string, string> = {};
      if (referer) reqHeaders.Referer = referer;
      if (cookie) reqHeaders.Cookie = cookie;
      if (authorization) reqHeaders.Authorization = authorization;

      try {
        const result = await api.precheck({
          url: firstUrl,
          target_directory: destination || undefined,
          suggested_filename: userEditedFileName.current ? (fileName || undefined) : undefined,
          headers: Object.keys(reqHeaders).length > 0 ? reqHeaders : undefined,
        });
        if (cancelled || seq !== precheckSeqRef.current) return;
        setPrecheck(result);
        if (!userEditedConnections.current && result.suggested_connections) {
          setConnections(result.suggested_connections);
        }
        // 用户未手动编辑文件名时，先清理再用结果预填文件名。
        const finalFileName = result.file_name && cleanupRules.length > 0
          ? applyFilenameCleanup(result.file_name, cleanupRules)
          : result.file_name;
        const effectiveFileName = userEditedFileName.current
          ? fileName
          : (finalFileName || fileName);
        if (!userEditedFileName.current && finalFileName) {
          setFileName(finalFileName);
        }
        // SubTask 10.5：调用重复检测（与预检同步触发，避免增加 debounce）。
        try {
          const sep = destination.endsWith("/") || destination.endsWith("\\") ? "" : "/";
          const targetPath = effectiveFileName
            ? `${destination}${sep}${effectiveFileName}`
            : destination;
          const dup = await api.duplicateCheck(firstUrl, targetPath, {
            fileSize: result.file_size,
          });
          if (!cancelled && seq === precheckSeqRef.current) setDuplicateResult(dup);
        } catch {
          if (!cancelled && seq === precheckSeqRef.current) setDuplicateResult(undefined);
        }
      } catch (err) {
        if (cancelled || seq !== precheckSeqRef.current) return;
        setPrecheckError(String(err));
      } finally {
        if (!cancelled && seq === precheckSeqRef.current) setPrecheckLoading(false);
      }
    }, 400);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [urls, destination, referer, cookie, authorization]);

  // 手动触发预检（PrecheckPanel 刷新回调）
  const runPrecheckNow = useCallback(() => {
    const firstUrl = lines[0];
    if (!firstUrl || lines.length !== 1 || !isDownloadableUrlForDialog(firstUrl)) return;
    const seq = ++precheckSeqRef.current;
    setPrecheckLoading(true);
    setPrecheckError(undefined);

    const reqHeaders: Record<string, string> = {};
    if (referer) reqHeaders.Referer = referer;
    if (cookie) reqHeaders.Cookie = cookie;
    if (authorization) reqHeaders.Authorization = authorization;

    void (async () => {
      try {
        const result = await api.precheck({
          url: firstUrl,
          target_directory: destination || undefined,
          suggested_filename: userEditedFileName.current ? (fileName || undefined) : undefined,
          headers: Object.keys(reqHeaders).length > 0 ? reqHeaders : undefined,
        });
        if (seq !== precheckSeqRef.current) return;
        setPrecheck(result);
        if (!userEditedConnections.current && result.suggested_connections) {
          setConnections(result.suggested_connections);
        }
        const finalFileName = result.file_name && cleanupRules.length > 0
          ? applyFilenameCleanup(result.file_name, cleanupRules)
          : result.file_name;
        const effectiveFileName = userEditedFileName.current
          ? fileName
          : (finalFileName || fileName);
        if (!userEditedFileName.current && finalFileName) {
          setFileName(finalFileName);
        }
        try {
          const sep = destination.endsWith("/") || destination.endsWith("\\") ? "" : "/";
          const targetPath = effectiveFileName
            ? `${destination}${sep}${effectiveFileName}`
            : destination;
          const dup = await api.duplicateCheck(firstUrl, targetPath, {
            fileSize: result.file_size,
          });
          if (seq === precheckSeqRef.current) setDuplicateResult(dup);
        } catch {
          if (seq === precheckSeqRef.current) setDuplicateResult(undefined);
        }
      } catch (err) {
        if (seq !== precheckSeqRef.current) return;
        setPrecheckError(String(err));
      } finally {
        if (seq === precheckSeqRef.current) setPrecheckLoading(false);
      }
    })();
  }, [lines, destination, referer, cookie, authorization, fileName]);

  // 给文件名追加后缀以避免冲突（重新下载场景）。
  const suffixedFileName = (name: string): string => {
    if (!name) return `download-${Date.now()}.bin`;
    const dotIndex = name.lastIndexOf(".");
    if (dotIndex <= 0) return `${name}-${Date.now()}`;
    return `${name.slice(0, dotIndex)}-${Date.now()}${name.slice(dotIndex)}`;
  };


  // 结合 ignoreUrlConflict 与当前 fileName 动态计算有效冲突列表
  const activeConflicts = useMemo(() => {
    if (!precheck?.conflicts?.length) return [];
    const sep = destination.endsWith("/") || destination.endsWith("\\") ? "" : "/";
    const currentTargetPath = (destination && fileName) ? `${destination}${sep}${fileName}`.toLowerCase() : "";

    return precheck.conflicts.filter((conflict) => {
      if (ignoreUrlConflict && (conflict.conflict_type === "duplicate-url" || conflict.conflict_type === "duplicate-final-url")) {
        return false;
      }
      if (conflict.conflict_type === "duplicate-target-path") {
        if (!currentTargetPath || !allTasks?.length) return true;
        return allTasks.some((t) => {
          if (t.status === "cancelled") return false;
          const tPath = `${t.destination}${t.destination.endsWith("/") || t.destination.endsWith("\\") ? "" : "/"}${t.file_name}`.toLowerCase();
          return tPath === currentTargetPath;
        });
      }
      return true;
    });
  }, [precheck?.conflicts, ignoreUrlConflict, destination, fileName, allTasks]);

  const hasConflicts = activeConflicts.length > 0;
  const hasDuplicates = Boolean(duplicateResult?.matches?.length);
  // Task 42 / Task 47：图集与合集场景下，提交按钮需在未选中任何子项时禁用。
  const isGalleryWithoutSelection =
    (media?.media_type === "gallery" && selectedImageIds.size === 0) ||
    (media?.media_type === "collection" && selectedEpisodeIndices.size === 0);
  const showConflictOptions = hasConflicts && !hasDuplicates;

  // 聚合当前目标盘同卷已排队/下载任务的空间需求
  const { queueDiskTotal, queueUnknownCount } = useMemo(() => {
    if (!allTasks?.length || !destination) return { queueDiskTotal: 0, queueUnknownCount: 0 };
    const getVolume = (p: string) => {
      const norm = p.replace(/\//g, "\\");
      const match = /^([a-zA-Z]:)/.exec(norm);
      return match ? match[1].toUpperCase() : "\\";
    };
    const targetVolume = getVolume(destination);
    let total = 0;
    let unknownCount = 0;
    const activeStatuses = new Set(["downloading", "queued", "scheduled", "verifying", "paused-by-low-disk", "waiting-network"]);

    for (const task of allTasks) {
      if (!activeStatuses.has(task.status)) continue;
      if (getVolume(task.destination) !== targetVolume) continue;
      if (task.total_bytes > 0) {
        const remaining = task.total_bytes > task.downloaded_bytes ? task.total_bytes - task.downloaded_bytes : 0;
        const isMulti = task.connection_count > 1;
        total += isMulti ? (remaining + task.total_bytes + 100 * 1024 * 1024) : (remaining + 50 * 1024 * 1024);
      } else {
        unknownCount++;
      }
    }
    return { queueDiskTotal: total, queueUnknownCount: unknownCount };
  }, [allTasks, destination]);

  // Task 12: 把预设的 "HH:MM" 字符串转换为下一次该时刻的 datetime-local 字符串。
  // 浏览器 <input type="datetime-local"> 期望格式 "YYYY-MM-DDTHH:MM"。
  const hhmmToNextDatetimeLocal = (hhmm: string): string => {
    const match = /^(\d{1,2}):(\d{2})$/.exec(hhmm.trim());
    if (!match) return "";
    const hour = Number(match[1]);
    const minute = Number(match[2]);
    if (hour > 23 || minute > 59) return "";
    const now = new Date();
    const target = new Date(now.getFullYear(), now.getMonth(), now.getDate(), hour, minute, 0, 0);
    if (target.getTime() <= now.getTime()) {
      target.setDate(target.getDate() + 1);
    }
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${target.getFullYear()}-${pad(target.getMonth() + 1)}-${pad(target.getDate())}T${pad(hour)}:${pad(minute)}`;
  };

  // Task 12: 应用预设到表单字段（连接数、限速、完成动作、计划时间）。
  // verify_checksum 在 NewTaskRequest 中无对应字段，由 preset_apply_to_task 在任务创建后应用。
  const applyPreset = (preset: DownloadPreset | undefined) => {
    if (!preset) {
      setSelectedPresetId("");
      return;
    }
    setSelectedPresetId(preset.id);
    setConnections(preset.connections);
    setLimit(preset.speed_limit ? Math.round(preset.speed_limit / 1024) : 0);
    setCompletionAction(preset.completion_action ?? "none");
    setSchedule(preset.scheduled_at ? hhmmToNextDatetimeLocal(preset.scheduled_at) : "");
  };

  const probe = async () => {
    setBusy(true); setError(undefined);
    try {
      const result = await api.probeMedia(lines[0], { cookie: cookie || undefined, referer: referer || undefined });
      if (result.drm) throw new Error("检测到 DRM 保护，猫步下载器不处理此内容");
      setMedia(result);
      // Task 42：根据 media_type 选择默认格式
      // - Video / Mixed：选择最高画质的视频流（与历史行为一致）
      // - Audio：仅显示音频流（has_audio && !has_video），按高度/比特率排序后选最佳
      // - Gallery：默认不选视频格式，前端展示图片网格供用户多选
      if (result.media_type === "gallery") {
        setFormat("");
        // 图集默认全选所有图片项
        const imageItems = result.formats.filter((item) => item.image_url);
        setSelectedImageIds(new Set(imageItems.map((item) => item.id)));
        if (!fileName) setFileName(safeDisplayName(result.title));
      } else if (result.media_type === "collection") {
        setFormat("");
        // 合集/多 P 默认全选所有分 P
        const eps = result.episodes || [];
        setSelectedEpisodeIndices(new Set(eps.map((e) => e.index)));
        if (!fileName) setFileName(safeDisplayName(result.title));
      } else if (result.media_type === "audio") {
        const audioFormats = result.formats
          .filter((item) => item.has_audio && !item.has_video)
          .sort((a, b) => (b.file_size ?? 0) - (a.file_size ?? 0));
        const selected = audioFormats[0] ?? result.formats[0];
        setFormat(selected?.id ?? "");
        if (!fileName) setFileName(`${safeDisplayName(result.title)}.m4a`);
      } else {
        // video / mixed：默认格式选择优先级：
        // 1. 合并格式（has_video && has_audio && requires_ffmpeg）—— 最高画质原始流合并
        //    Twitter/X 的 hls-* 原始流画质优于 http-* progressive 二次压缩流，
        //    yt-dlp 默认也推荐 bestvideo*+bestaudio/best 合并格式。
        //    后端在 FFmpeg 可用时调用 FFmpeg 合并，FFmpeg 不可用时调用内置 media_muxer
        //    （纯 Rust 实现的 fMP4 合并器）合并，因此前端无需根据 FFmpeg 状态切换默认格式。
        // 2. 有声视频直链（has_video && has_audio && !requires_ffmpeg）—— progressive 流回退
        // 3. 无声视频（has_video && !has_audio && !requires_ffmpeg）—— 最后回退
        // 4. 其它首项
        const hasFfmpeg = toolStatus?.ffmpeg_available ?? false;
        const directOrVideo = result.formats
          .filter((item) => item.has_video && !item.requires_ffmpeg)
          .sort((a, b) => (b.height ?? 0) - (a.height ?? 0));
        const merged = hasFfmpeg
          ? result.formats.filter((item) => item.has_video && item.has_audio && item.requires_ffmpeg).sort((a, b) => (b.height ?? 0) - (a.height ?? 0))
          : [];
        const selected = directOrVideo[0] ?? merged[0] ?? result.formats[0];
        setFormat(selected?.id ?? "");
        if (!fileName) setFileName(`${safeDisplayName(result.title)}.mp4`);
      }
    } catch (reason) {
      const text = String(reason);
      if (text.includes("MEDIA_YT_DLP_MISSING")) setToolStatus(await api.toolStatus());
      else setError(text);
    } finally { setBusy(false); }
  };
  const performSubmit = async (overrideFileName?: string) => {
    if (!lines.length) return; setBusy(true); setError(undefined);
    const activeFileName = overrideFileName !== undefined ? overrideFileName : fileName;
    if (precheck?.disk_state === "insufficient" || (precheck && !precheck.disk_ok)) {
      const confirmed = window.confirm(
        `检测到目标磁盘可用空间不足（可用 ${formatBytes(precheck.available_disk_bytes)}，计算所需 ${formatBytes(precheck.required_disk_bytes)}）。\n\n强行开始可能导致任务在下载过程中自动暂停。是否仍要尝试开始下载？`
      );
      if (!confirmed) {
        setBusy(false);
        return;
      }
    }
    const headers: Record<string, string> = {}; if (referer) headers.Referer = referer; if (cookie) headers.Cookie = cookie; if (authorization) headers.Authorization = authorization;
    const selectedFormat = media?.formats.find((item) => item.id === format);
    if (selectedFormat?.requires_ffmpeg && !toolStatus?.ffmpeg_available) {
      setError("当前最高画质需要先安装 FFmpeg 高清合并组件");
      setBusy(false);
      return;
    }
    // Task 47：合集/多 P 类型，每个选中的分 P 作为独立子任务。
    // 文件名按 `{合集名称}_P{序号}_{分P标题}.mp4` 模式，自动排队下载。
    if (media?.media_type === "collection") {
      const eps = (media.episodes || []).filter((e) => selectedEpisodeIndices.has(e.index));
      if (eps.length === 0) {
        setError("请至少选择一集再开始下载");
        setBusy(false);
        return;
      }
      const collectionTitleBase = safeDisplayName(activeFileName || media.title);
      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination, headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority, expected_checksum: checksum || undefined, source: "desktop",
        per_task_speed_limit: limit * 1024, collision_policy: policy,
        completion_action: eps.length > 1 ? "none" : completionAction,
        connection_count: connections,
        media: undefined,
        user_edited_file_name: userEditedFileName.current || overrideFileName !== undefined,
      };
      try {
        const results = await Promise.allSettled(
          eps.map((ep) => {
            const epTitle = safeDisplayName(ep.title);
            const file_name = `${collectionTitleBase}_P${ep.index}_${epTitle}.mp4`;
            return api.add({ url: ep.url, file_name, ...baseTemplate });
          })
        );
        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          setError(
            fulfilled.length === 0
              ? firstError
              : `部分集数创建失败：${firstError}（成功 ${fulfilled.length}/${eps.length}）`
          );
        }
      } catch (reason) { setError(String(reason)); }
      setBusy(false);
      return;
    }
    // Task 42：图集类型，每张选中的图片作为独立子任务。
    // 文件名按 `{title}_{index}.{ext}` 模式，1-indexed。直链图片不走 yt-dlp，
    // 由 HTTP Range 多连接内核处理（AGENTS.md §3）。
    if (media?.media_type === "gallery") {
      const imageItems = media.formats.filter(
        (item) => item.image_url && selectedImageIds.has(item.id)
      );
      if (imageItems.length === 0) {
        setError("请至少选择一张图片再开始下载");
        setBusy(false);
        return;
      }
      const titleBase = safeDisplayName(activeFileName || media.title);
      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination, headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority, expected_checksum: checksum || undefined, source: "desktop",
        per_task_speed_limit: limit * 1024, collision_policy: policy,
        // 图集多图批量场景下关闭单任务完成动作，避免触发 N 次关机/打开文件夹。
        completion_action: imageItems.length > 1 ? "none" : completionAction,
        connection_count: connections,
        // 直链图片无需 yt-dlp 媒体信息
        media: undefined,
        user_edited_file_name: userEditedFileName.current || overrideFileName !== undefined,
      };
      try {
        const results = await Promise.allSettled(
          imageItems.map((item, index) => {
            const ext = (item.extension || "jpg").replace(/^\./, "").toLowerCase();
            const file_name = `${titleBase}_${index + 1}.${ext}`;
            return api.add({ url: item.image_url!, file_name, ...baseTemplate });
          })
        );
        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        // 图集页面 URL 写入历史一次（不是每张图直链）
        void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          setError(
            fulfilled.length === 0
              ? firstError
              : `部分图片创建失败：${firstError}（成功 ${fulfilled.length}/${imageItems.length}）`
          );
        }
      } catch (reason) { setError(String(reason)); }
      setBusy(false);
      return;
    }
    const template: Omit<NewTaskRequest, "url"> = { file_name: activeFileName || undefined, destination, headers, scheduled_at: schedule ? new Date(schedule).getTime() : undefined, priority, expected_checksum: checksum || undefined, source: "desktop", per_task_speed_limit: limit * 1024, collision_policy: policy, completion_action: lines.length > 1 ? "none" : completionAction, connection_count: connections, media: media ? { extractor: media.extractor, format_id: format, format_label: selectedFormat?.label, subtitles: [], thumbnail: media.thumbnail, requires_ffmpeg: selectedFormat?.requires_ffmpeg } : undefined, user_edited_file_name: userEditedFileName.current || overrideFileName !== undefined };
    try {
      // Task 19: 创建任务成功后将 URL 写入历史（LRU），失败不阻塞主流程。
      // 多行批量场景下逐条写入；单行场景只写一次。
      if (lines.length === 1) {
        const task = await api.add({ url: lines[0], ...template });
        void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        onCreated(task);
      } else {
        const tasks = await api.addBatch(lines, template);
        // 串行写入历史，避免并发竞争同一行（虽然 SQL upsert 安全，但保持顺序更可读）。
        void (async () => {
          for (const url of lines) {
            try { await api.urlHistoryAdd(url); } catch { /* 历史写入失败忽略 */ }
          }
          reloadHistory();
        })();
        onCreated(tasks);
      }
    } catch (reason) { setError(String(reason)); setBusy(false); }
  };

  const submit = () => performSubmit();

  const handleRedownloadDirectly = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const allToId = new Set<string>();
      if (duplicateResult?.matches) {
        for (const m of duplicateResult.matches) {
          allToId.add(m.existing_task_id);
        }
      }
      if (precheck?.conflicts) {
        for (const c of precheck.conflicts) {
          allToId.add(c.existing_task_id);
        }
      }
      if (activeConflicts) {
        for (const c of activeConflicts) {
          allToId.add(c.existing_task_id);
        }
      }

      for (const id of allToId) {
        try {
          await api.remove(id, true);
        } catch (e) {
          console.error("Failed to remove duplicate task:", id, e);
        }
      }

      setDuplicateResult(undefined);
      setPrecheck(prev => prev ? { ...prev, conflicts: [] } : undefined);
      setIgnoreUrlConflict(true);

      await performSubmit();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  const handleRenameAndSubmit = async () => {
    const next = suffixedFileName(fileName || precheck?.file_name || "");
    setFileName(next);
    userEditedFileName.current = true;

    setDuplicateResult(undefined);
    setPrecheck(prev => prev ? { ...prev, conflicts: [] } : undefined);
    setIgnoreUrlConflict(true);

    await performSubmit(next);
  };
  return (
    <Modal title="新建下载任务" onClose={onClose} style={{ display: "flex", flexDirection: "column", height: "560px", maxHeight: "calc(100vh - 80px)", overflow: "hidden" }}>
      <div className="new-task-form" style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "hidden" }}>
        <div className="new-task-scrollable" style={{ flex: 1, overflowY: "auto", overflowX: "hidden", paddingRight: "6px", display: "flex", flexDirection: "column", gap: "11px" }}>
          {/* Task 12: 下载预设选择下拉。选择后自动填充连接数/限速/完成动作/计划时间。 */}
        {presets.length > 0 && (
          <div className="form-group-row">
            <label className="form-field grow">
              <span>下载预设</span>
              <div className="input-group">
                <Select
                  value={selectedPresetId}
                  onChange={(val: any) => {
                    const id = String(val);
                    setSelectedPresetId(id);
                    const preset = presets.find((p) => p.id === id);
                    applyPreset(preset);
                  }}
                  options={[
                    { value: "", label: "不使用预设" },
                    ...presets.map((p) => ({
                      value: p.id,
                      label: `${p.name}${p.is_builtin ? "（内置）" : ""} · ${p.connections} 连接${p.speed_limit ? ` · 限速 ${Math.round(p.speed_limit / 1024)} KB/s` : ""}${p.completion_action && p.completion_action !== "none" ? ` · ${p.completion_action === "open-folder" ? "打开文件夹" : p.completion_action === "run-file" ? "运行文件" : p.completion_action === "shutdown" ? "完成后关机" : "完成后休眠"}` : ""}${p.scheduled_at ? ` · 计划 ${p.scheduled_at}` : ""}`,
                    })),
                  ]}
                  ariaLabel="选择下载预设"
                />
              </div>
            </label>
          </div>
        )}
        <div className="form-section">
          <label className="form-field url-input-field">
            <div className="form-label-bar">
              <span>下载链接（每行一个）</span>
              <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                {(() => {
                  if (!normalizedUrlPreview || lines.length !== 1) return null;
                  const firstUrl = lines[0];
                  const isPurified = normalizedUrlPreview.trim().toLowerCase() !== firstUrl.trim().toLowerCase();
                  return (
                    <span
                      className="normalized-badge"
                      title={isPurified ? `链接已自动净化（双击复制完整链接）：\n${normalizedUrlPreview}` : `链接已成功解析并验证（双击复制完整链接）：\n${normalizedUrlPreview}`}
                      style={{
                        display: "inline-flex",
                        alignItems: "center",
                        gap: "2px",
                        padding: "1px 5px",
                        background: isPurified ? "var(--success-bg, rgba(52, 199, 89, 0.12))" : "rgba(0, 120, 212, 0.08)",
                        color: isPurified ? "var(--success, #34c759)" : "var(--accent, #0078d4)",
                        borderRadius: "3px",
                        fontSize: "9px",
                        fontWeight: "normal",
                        cursor: "pointer",
                        border: isPurified ? "1px solid rgba(52, 199, 89, 0.2)" : "1px solid rgba(0, 120, 212, 0.15)"
                      }}
                      onDoubleClick={(e) => {
                        e.stopPropagation();
                        void navigator.clipboard.writeText(normalizedUrlPreview);
                        notify?.("已复制规范化 URL 到剪贴板", "ok");
                      }}
                    >
                      <Check size={8} strokeWidth={3} />
                      {isPurified ? "链接已净化" : "链接已解析"}
                    </span>
                  );
                })()}
                {lines.length > 0 && (
                  <span className="form-label-counter">
                    {lines.length > 1
                      ? `检测到 ${lines.length} 条 URL，将批量创建`
                      : `已检测到 ${lines.length} 个链接`}
                  </span>
                )}
              </div>
            </div>
            <div className="url-input-wrap">
              <textarea
                autoFocus
                value={urls}
                onFocus={() => setHistoryOpen(true)}
                onBlur={() => {
                  // 延迟关闭，让点击事件先触发
                  window.setTimeout(() => setHistoryOpen(false), 180);
                }}
                onChange={(e) => {
                  const val = e.target.value;
                  setUrls(val);
                  setMedia(undefined);
                  // Task 42：URL 变化时重置图集选中状态，避免上一图集的选中残留。
                  setSelectedImageIds(new Set());
                  // URL 变化时重置"用户已手动编辑文件名"标记，允许预检结果覆盖空文件名。
                  userEditedFileName.current = false;
                  const parsed = parseMultilineUrls(val);
                  if (parsed.lines.length === 1) {
                    const name = extractFileNameFromUrl(parsed.lines[0]);
                    if (name) {
                      setFileName(name);
                    }
                  } else if (parsed.lines.length === 0) {
                    setFileName("");
                  }
                }}
                placeholder="https://example.com/file.zip"
                aria-label="下载链接，支持多行批量"
              />
              {/* Task 19: 历史下拉。聚焦时显示最近 20 条 URL 历史，点击填充到输入框。 */}
              {historyOpen && urlHistory.length > 0 && (
                <div className="url-history-dropdown" role="listbox" aria-label="最近 URL 历史">
                  <div className="url-history-header">
                    <span>最近 URL</span>
                    <button
                      type="button"
                      className="url-history-clear"
                      onMouseDown={(e) => {
                        // 阻止 blur 抢先关闭下拉
                        e.preventDefault();
                        void api.urlHistoryClear().then(() => {
                          setUrlHistory([]);
                        }).catch(() => {});
                      }}
                      title="清空全部历史"
                    >
                      <Trash2 size={11} />
                      <span>清空</span>
                    </button>
                  </div>
                  <ul className="url-history-list">
                    {urlHistory.map((entry) => (
                      <li key={entry.url}>
                        <button
                          type="button"
                          className="url-history-item"
                          onMouseDown={(e) => {
                            // 阻止 blur，使点击立即填充并保持焦点
                            e.preventDefault();
                            setUrls(entry.url);
                            setHistoryOpen(false);
                            setMedia(undefined);
                            userEditedFileName.current = false;
                            const name = extractFileNameFromUrl(entry.url);
                            if (name) setFileName(name);
                          }}
                          title={entry.url}
                        >
                          <Globe2 size={11} />
                          <span className="url-history-text">{entry.url}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
            {(skippedCount > 0 || duplicateCount > 0) && (
              <div className="url-parse-hint">
                {skippedCount > 0 && <span>已忽略 {skippedCount} 行非 HTTP/HTTPS 内容</span>}
                {skippedCount > 0 && duplicateCount > 0 && <span> · </span>}
                {duplicateCount > 0 && <span>已去重 {duplicateCount} 条重复 URL</span>}
              </div>
            )}
            {templateMatch?.matched && (
              <div className="url-template-hint" title="任务模板将自动套用到未由用户显式设置的字段">
                <Bookmark size={11} />
                <span>已匹配模板：<code>{templateMatch.matched_template_name ?? templateMatch.matched_template_id}</code></span>
              </div>
            )}
            {matchedCredentialDomain && (
              <div className="url-template-hint" title={`已保存的 ${matchedCredentialDomain} 凭证已应用`} role="status">
                <ShieldCheck size={11} />
                <span>已保存的 <code>{matchedCredentialDomain}</code> 凭证已应用</span>
              </div>
            )}
            {detectedPlatform && detectedPlatform !== "unknown" && (
              <div className="url-platform-hint" title="已识别媒体平台">
                <Globe2 size={11} />
                <span>检测到：{mediaPlatformDisplayName(detectedPlatform)}</span>
                {platformCompat && (
                  <span
                    className={`platform-badge platform-badge-${platformCompat.level}`}
                    title={platformCompat.notes || supportLevelLabel(platformCompat.level)}
                    style={{
                      marginLeft: 6,
                      padding: "1px 6px",
                      borderRadius: 4,
                      fontSize: 11,
                      color: "#fff",
                      backgroundColor: supportLevelColor(platformCompat.level),
                      border: `1px solid ${supportLevelColor(platformCompat.level)}`,
                    }}
                  >
                    {supportLevelLabel(platformCompat.level)}
                  </span>
                )}
              </div>
            )}
            {platformCompat && platformCompat.level === "unsupported" && (
              <div className="url-platform-hint" title="该平台暂不支持下载，请使用浏览器原生下载" role="alert">
                <AlertCircle size={11} />
                <span>该平台暂不支持下载，已禁用下载按钮</span>
              </div>
            )}
            {platformCompat && platformCompat.notes && platformCompat.level !== "unsupported" && (
              <div className="url-platform-hint" title={platformCompat.notes} role="status">
                <Info size={11} />
                <span>{platformCompat.notes}</span>
              </div>
            )}
            {detectedPlatform === "twitter" && (
              <div className="url-platform-hint" title="Twitter/X 通常需要登录态才能解析视频与 Spaces 音频">
                <ShieldCheck size={11} />
                <span>需要登录态，请使用扩展临时登录态或填写 Cookie</span>
              </div>
            )}

          </label>
        </div>

        <div className="form-group-row">
          <label className="form-field grow">
            <span>保存位置</span>
            <div className="input-group">
              <input
                value={destination}
                onChange={(e) => {
                  setDestination(e.target.value);
                  userEditedDestination.current = true;
                }}
              />
              <button
                className="input-button primary-border"
                onClick={async () => {
                  const path = await pickPath({
                    directory: true,
                    multiple: false,
                    defaultPath: destination,
                  });
                  if (typeof path === "string") {
                    setDestination(path);
                    userEditedDestination.current = true;
                  }
                }}
              >
                <FolderOpen size={13} />
                <span>浏览</span>
              </button>
            </div>
          </label>
        </div>

        <div className="form-grid-2">
          <div className="form-field">
            <div className="field-label-row">
              <span>分段连接数</span>
              <span className="field-label-value"><span className="field-label-num">{connections}</span><span className="field-label-text">路并发</span></span>
            </div>
            <div className="slider-container">
              <input
                type="range"
                min="0"
                max="5"
                step="1"
                value={[1, 2, 4, 8, 16, 32].indexOf(connections)}
                onChange={(e) => {
                  const values = [1, 2, 4, 8, 16, 32];
                  userEditedConnections.current = true;
                  setConnections(values[+e.target.value]);
                }}
                className="fluent-slider"
              />
              <div className="slider-ticks">
                <span>1</span>
                <span>2</span>
                <span>4</span>
                <span>8</span>
                <span>16</span>
                <span>32</span>
              </div>
            </div>
          </div>

          <div className="form-field">
            <div className="field-label-row">
              <span>重名处理</span>
            </div>
            <div className="fluent-segmented-control">
              <button
                type="button"
                className={policy === "rename" ? "active" : ""}
                onClick={() => setPolicy("rename")}
              >
                重命名
              </button>
              <button
                type="button"
                className={policy === "overwrite" ? "active" : ""}
                onClick={() => setPolicy("overwrite")}
              >
                覆盖
              </button>
              <button
                type="button"
                className={policy === "skip" ? "active" : ""}
                onClick={() => setPolicy("skip")}
              >
                跳过
              </button>
            </div>
            <div className="field-helper-text">
              {policy === "rename" && "当文件名存在冲突时自动追加数字后缀"}
              {policy === "overwrite" && "直接覆盖同名文件，旧文件将被完全替换"}
              {policy === "skip" && "跳过该任务的下载，直接保留本地文件"}
            </div>
          </div>
        </div>

        {lines.length === 1 && (
          <div className="form-group-row">
            <label className="form-field grow">
              <span>文件名（可选）</span>
              <div className="input-group">
                <input
                  ref={fileNameInputRef}
                  value={fileName}
                  onChange={(e) => {
                    userEditedFileName.current = true;
                    setFileName(e.target.value);
                  }}
                  placeholder="保持默认（根据服务器响应解析）"
                />
                <button
                  className="input-button media-probe-btn"
                  disabled={busy}
                  onClick={() => void probe()}
                >
                  <Video size={13} />
                  <span>{busy ? "正在分析..." : "分析媒体"}</span>
                </button>
              </div>

            </label>
          </div>
        )}

        {toolStatus && (!toolStatus.yt_dlp_available || (media?.formats.find((item) => item.id === format)?.requires_ffmpeg && !toolStatus.ffmpeg_available)) && (
          <MediaToolsCard status={toolStatus} compact required={!toolStatus.yt_dlp_available ? "yt-dlp" : "ffmpeg"} onStatus={setToolStatus} />
        )}

        {media && (
          <div className="media-result-card">
            <div className="media-result-header">
              <span className="media-tag">已探测媒体</span>
              <strong>{media.title}</strong>
            </div>
            {media.media_type === "gallery" ? (
              <GalleryPicker
                formats={media.formats}
                thumbnail={media.thumbnail}
                selectedIds={selectedImageIds}
                onChange={setSelectedImageIds}
              />
            ) : media.media_type === "collection" ? (
              <EpisodePicker
                episodes={media.episodes || []}
                selectedIndices={selectedEpisodeIndices}
                onChange={setSelectedEpisodeIndices}
                qualityPreference={collectionQualityPreference}
                onQualityChange={setCollectionQualityPreference}
              />
            ) : media.media_type === "audio" ? (
              <div className="media-format-select-row">
                <Select
                  value={format}
                  onChange={(val: any) => setFormat(String(val))}
                  options={media.formats
                    .filter((item) => item.has_audio && !item.has_video)
                    .map((item) => ({
                      value: item.id,
                      label: `${item.label}${item.file_size ? ` (${formatBytes(item.file_size)})` : ""}`,
                    }))}
                  ariaLabel="音频格式选择"
                  style={{ width: "100%" }}
                />
                {media.formats.filter((item) => item.has_audio && !item.has_video).length === 0 && (
                  <div className="media-empty-hint">未识别到独立音频流，将尝试使用默认格式下载</div>
                )}
              </div>
            ) : (
              <div className="media-format-select-row">
                <Select
                  value={format}
                  onChange={(val: any) => setFormat(String(val))}
                  options={media.formats
                    .filter((item) => item.has_video || item.has_audio)
                    .map((item) => ({
                      value: item.id,
                      label: `${item.label}${item.file_size ? ` (${formatBytes(item.file_size)})` : ""}${!item.requires_ffmpeg && item.has_video && item.has_audio ? " · 轻量单文件" : ""}`,
                    }))}
                  ariaLabel="视频格式选择"
                  style={{ width: "100%" }}
                />
              </div>
            )}
          </div>
        )}

        <div className="advanced-divider">
          <button
            className={advanced ? "advanced-toggle active" : "advanced-toggle"}
            onClick={() => setAdvanced((value) => !value)}
          >
            <ChevronDown size={13} />
            <span>高级下载选项</span>
          </button>
        </div>

        {advanced && (
          <div className="advanced-options-panel">
            <div className="advanced-grid">
              <Field label="计划开始时间">
                <input
                  type="datetime-local"
                  value={schedule}
                  onChange={(e) => setSchedule(e.target.value)}
                />
              </Field>
              <Field label="单任务限速">
                <div className="input-with-unit">
                  <input
                    type="number"
                    min="0"
                    value={limit}
                    onChange={(e) => setLimit(+e.target.value)}
                    placeholder="0 表示不限制"
                  />
                  <span className="unit-label">KB/s</span>
                </div>
              </Field>
              <Field label="任务优先级（排队与带宽）">
                <Select
                  value={priority}
                  onChange={(val: any) => setPriority(+val)}
                  options={[
                    { value: TASK_PRIORITY_PRESETS.high, label: "高优先级" },
                    { value: TASK_PRIORITY_PRESETS.normal, label: "普通" },
                    { value: TASK_PRIORITY_PRESETS.low, label: "低优先级" },
                  ]}
                  ariaLabel="任务优先级"
                  style={{ width: "100%" }}
                />
              </Field>
              <Field label="自定义 Referer">
                <input
                  value={referer}
                  onChange={(e) => setReferer(e.target.value)}
                  placeholder="https://..."
                />
              </Field>
              <Field label="自定义 Cookie">
                <input
                  value={cookie}
                  onChange={(e) => setCookie(e.target.value)}
                  placeholder="key=value; ..."
                />
              </Field>
              <Field label="自定义 Authorization 头部">
                <input
                  value={authorization}
                  onChange={(e) => setAuthorization(e.target.value)}
                  placeholder="Bearer ... 或 Basic ..."
                />
              </Field>
              <Field className={["run-command", "copy-to", "move-to"].includes(completionActionKind(completionAction)) ? "wide" : ""} label="预期文件 SHA-256">
                <input
                  value={checksum}
                  onChange={(e) => setChecksum(e.target.value)}
                  placeholder="用于校验文件完整性"
                />
              </Field>
              <Field className={["run-command", "copy-to", "move-to"].includes(completionActionKind(completionAction)) ? "wide" : ""} label="下载完成后">
                <CompletionActionEditor
                  value={completionAction}
                  onChange={setCompletionAction}
                  allowRunFile={lines.length === 1}
                />
              </Field>
            </div>
          </div>
        )}

        {/* 预检结果面板（移至表单最下方） */}
        {lines.length === 1 && isDownloadableUrlForDialog(lines[0]) && (
          <PrecheckPanel
            result={precheck}
            loading={precheckLoading}
            error={precheckError}
            queueDiskTotal={queueDiskTotal}
            queueUnknownCount={queueUnknownCount}
            onLocateConflict={(conflict) => onLocateTask?.(conflict.existing_task_id)}
            onRefresh={runPrecheckNow}
          />
        )}

        {error && <div className="inline-error">{error}</div>}


        </div>

        <div className="new-task-sticky-footer" style={{ flexShrink: 0, marginTop: "12px", borderTop: "1px solid var(--border-strong)", paddingTop: "12px", display: "flex", flexDirection: "column", gap: "10px" }}>
          {/* SubTask 9.2: 冲突四选项（仅在冲突命中且未命中重复时显示） */}
        {showConflictOptions && (
          <div className="conflict-options" role="group" aria-label="冲突处理">
            <div className="conflict-options-header">
              <AlertTriangle size={11} />
              <span>检测到与已有任务冲突，请选择处理方式：</span>
            </div>
            <div className="conflict-options-row">
              <button
                type="button"
                className="conflict-option"
                onClick={() => {
                  const first = activeConflicts[0] || precheck?.conflicts?.[0];
                  if (first) onLocateTask?.(first.existing_task_id);
                }}
                title="在主列表中选中已有任务"
              >
                <ExternalLink size={11} />
                <span>定位已有任务</span>
              </button>
              <button
                type="button"
                className="conflict-option"
                disabled={busy}
                onClick={handleRedownloadDirectly}
                title="删除已有的冲突任务与文件，并立即重新开始下载"
              >
                <Download size={11} />
                <span>重新下载</span>
              </button>
              <button
                type="button"
                className="conflict-option"
                onClick={handleRenameAndSubmit}
                title="自动在文件名后增加时间戳后缀以避免冲突，并立即开始下载"
              >
                <FileText size={11} />
                <span>改文件名</span>
              </button>
              <button
                type="button"
                className="conflict-option secondary"
                onClick={onClose}
                title="关闭对话框，不创建任务"
              >
                <X size={11} />
                <span>跳过</span>
              </button>
            </div>
          </div>
        )}

        {/* SubTask 10.5: 重复检测四选项（命中 duplicateCheck 时显示） */}
        {hasDuplicates && duplicateResult && (
          <div className="conflict-options" role="group" aria-label="重复任务处理">
            <div className="conflict-options-header">
              <AlertTriangle size={11} />
              <span>
                检测到与已有任务重复（{duplicateResult.matches.map((m) => duplicateTypeLabel[m.duplicate_type]).join("、")}），请选择处理方式：
              </span>
            </div>
            <ul className="duplicate-match-list">
              {duplicateResult.matches.map((m) => (
                <li key={`${m.duplicate_type}-${m.existing_task_id}`} className="duplicate-match-item">
                  <span className="duplicate-match-type">{duplicateTypeLabel[m.duplicate_type]}</span>
                  <span className="duplicate-match-label" title={m.existing_task_label}>{m.existing_task_label}</span>
                  <span className="duplicate-match-status">（{statusText[m.existing_task_status as TaskStatus] ?? m.existing_task_status}）</span>
                </li>
              ))}
            </ul>
            <div className="conflict-options-row">
              <button
                type="button"
                className="conflict-option"
                onClick={() => {
                  const first = duplicateResult.matches[0];
                  if (first) onLocateTask?.(first.existing_task_id);
                }}
                title="在主列表中选中已有任务"
              >
                <ExternalLink size={11} />
                <span>定位已有任务</span>
              </button>
              <button
                type="button"
                className="conflict-option"
                disabled={busy}
                onClick={handleRedownloadDirectly}
                title="删除已有的冲突任务与文件，并立即重新开始下载"
              >
                <Download size={11} />
                <span>重新下载</span>
              </button>
              <button
                type="button"
                className="conflict-option"
                onClick={handleRenameAndSubmit}
                title="自动在文件名后增加时间戳后缀以避免冲突，并立即开始下载"
              >
                <FileText size={11} />
                <span>改文件名</span>
              </button>
              <button
                type="button"
                className="conflict-option secondary"
                onClick={onClose}
                title="关闭对话框，不创建任务"
              >
                <X size={11} />
                <span>跳过</span>
              </button>
            </div>
          </div>
        )}

        <div className="dialog-actions new-task-actions">
          <button className="cancel-btn" onClick={onClose}>
            取消
          </button>
          <button
            className="primary confirm-btn"
            disabled={busy || !lines.length || hasConflicts || hasDuplicates || isGalleryWithoutSelection || platformCompat?.level === "unsupported"}
            title={platformCompat?.level === "unsupported" ? "该平台暂不支持下载，请使用浏览器原生下载" : hasConflicts || hasDuplicates ? "存在冲突或重复，请先选择处理方式" : isGalleryWithoutSelection ? "请至少选择一张图片" : undefined}
            onClick={() => void submit()}
          >
            {busy ? "正在提交任务..." : "开始下载"}
          </button>
        </div>
        </div>
      </div>
    </Modal>
  );
}

/** Task 21: 快捷键设置面板与按键录制组件。 */
function ShortcutSettingsSection({
  value,
  onChange,
  notify,
}: {
  value: ShortcutKeys;
  onChange: (value: ShortcutKeys) => void;
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [recordingAction, setRecordingAction] = useState<keyof ShortcutKeys | null>(null);

  const actionKeys: Array<{ key: keyof ShortcutKeys; label: string }> = [
    { key: "new_task", label: t("shortcuts.actionNewTask") },
    { key: "select_all", label: t("shortcuts.actionSelectAll") },
    { key: "copy_url", label: t("shortcuts.actionCopyUrl") },
    { key: "open_folder", label: t("shortcuts.actionOpenFolder") },
    { key: "toggle_pause", label: t("shortcuts.actionTogglePause") },
    { key: "rename_task", label: t("shortcuts.actionRenameTask") },
    { key: "delete_task", label: t("shortcuts.actionDeleteTask") },
    { key: "delete_file", label: t("shortcuts.actionDeleteFile") },
  ];

  // 检查按键冲突（不区分大小写）
  const conflicts = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const item of actionKeys) {
      const val = (value[item.key] || "").toLowerCase();
      if (!val) continue;
      const existing = map.get(val) || [];
      existing.push(item.label);
      map.set(val, existing);
    }
    const conflictMsgs: string[] = [];
    for (const [key, labels] of map.entries()) {
      if (labels.length > 1) {
        conflictMsgs.push(`${labels.join(" / ")} (${key.toUpperCase()})`);
      }
    }
    return conflictMsgs;
  }, [value]);

  const handleKeyDown = (actionKey: keyof ShortcutKeys, event: React.KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      setRecordingAction(null);
      return;
    }

    const nativeEvent = event.nativeEvent;
    const keyCombo = parseShortcutEvent(nativeEvent);

    // 如果只按下了修饰键本身（如只按下了 Ctrl），保留在录制状态等待主按键
    if (["Ctrl", "Shift", "Alt"].includes(keyCombo)) {
      return;
    }

    onChange({ ...value, [actionKey]: keyCombo });
    setRecordingAction(null);
  };

  const handleResetDefaults = () => {
    onChange(DEFAULT_SHORTCUTS);
    notify(t("shortcuts.resetDefaultsSuccess"));
  };

  return (
    <div className="shortcuts-settings-section">
      <div className="shortcuts-section-header">
        <p className="settings-note" style={{ margin: 0 }}>{t("shortcuts.desc")}</p>
        <button className="secondary reset-btn" type="button" onClick={handleResetDefaults}>
          <RotateCcw size={13} />
          <span>{t("shortcuts.resetDefaults")}</span>
        </button>
      </div>

      {conflicts.length > 0 && (
        <div className="shortcut-conflict-warning">
          <AlertCircle size={14} />
          <span>{t("shortcuts.conflictWarning", { keys: conflicts.join("; ") })}</span>
        </div>
      )}

      <div className="shortcuts-list">
        {actionKeys.map(({ key, label }) => {
          const isRecording = recordingAction === key;
          const currentCombo = value[key] || DEFAULT_SHORTCUTS[key];

          return (
            <div key={key} className="shortcut-item-row">
              <span className="shortcut-action-label">{label}</span>
              <div className="shortcut-recorder-box">
                <button
                  type="button"
                  className={`shortcut-recorder-btn ${isRecording ? "recording" : ""}`}
                  onClick={() => setRecordingAction(isRecording ? null : key)}
                  onKeyDown={(e) => isRecording && handleKeyDown(key, e)}
                  tabIndex={0}
                >
                  {isRecording ? t("shortcuts.recordingHint") : currentCombo}
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

type SettingsSection = "general" | "download" | "network" | "browser" | "media" | "rules" | "filename-cleanup" | "naming-template" | "presets" | "templates" | "tags" | "credentials" | "appearance" | "shortcuts" | "advanced" | "about";
function SettingsPage({ value, onChange, onClose, notify, totalSpeed = 0, activeCount = 0 }: { value: AppSettings; onChange: (value: AppSettings) => void; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void; totalSpeed?: number; activeCount?: number }) {
  // Task 33: 订阅 locale 变化，设置页文案同步刷新。
  useLocale();
  const appWindow = useMemo(() => isDesktop() ? getCurrentWindow() : null, []);
  const [draft, setDraft] = useState(value); const [section, setSection] = useState<SettingsSection>("general");
  const [pair, setPair] = useState<PairingInfo>(); const [tools, setTools] = useState<ToolStatus>();
  // Task 26.5：更新检查与扩展兼容性检查的状态（只检查不自动下载）。
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);
  const [extVersion, setExtVersion] = useState("");
  const [extChecking, setExtChecking] = useState(false);
  const [extResult, setExtResult] = useState<ExtensionCompatibilityResult | null>(null);
  // Task 34.3：应用信息（便携模式、版本、数据目录）。设置页关于分组加载时拉取一次。
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    let cancelled = false;
    api.appGetInfo().then((info) => {
      if (!cancelled) setAppInfo(info);
    }).catch(() => {
      // 拉取失败时静默处理，关于分组仅显示默认信息。
    });
    return () => { cancelled = true; };
  }, []);
  // Task 44：平台兼容性矩阵列表。设置页"关于 > 平台兼容性"子区域展示。
  // 后端 Store::open 自动 seed 6 条内置记录；用户修改不会被覆盖。
  const [platformCompatList, setPlatformCompatList] = useState<PlatformCompatibility[]>([]);
  useEffect(() => {
    let cancelled = false;
    api.platformCompatibilityList().then((list) => {
      if (!cancelled) setPlatformCompatList(list ?? []);
    }).catch(() => {
      // 拉取失败时静默处理，子区域仅显示空列表占位。
      if (!cancelled) setPlatformCompatList([]);
    });
    return () => { cancelled = true; };
  }, []);
  const checkAppUpdate = async () => {
    setUpdateChecking(true);
    try {
      const result = await api.appCheckUpdate();
      setUpdateResult(result);
      if (result.error) notify(result.error, "error");
      else if (result.has_update) notify("发现新版本，请前往 GitHub 获取更新");
      else notify("当前已是最新版本");
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setUpdateChecking(false);
    }
  };
  const checkExtCompat = async () => {
    const trimmed = extVersion.trim();
    if (!trimmed) { notify("请先填写扩展版本号", "error"); return; }
    setExtChecking(true);
    try {
      const result = await api.extensionCheckCompatibility(trimmed);
      setExtResult(result);
      notify(result.compatible ? "扩展与桌面端版本兼容" : result.message || "扩展版本不兼容", result.compatible ? "ok" : "error");
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setExtChecking(false);
    }
  };
  const exportTasks = async () => {
    try {
      const path = await savePath({ defaultPath: "maobu-tasks.json", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (!path) return;
      const count = await api.exportTasks(path);
      notify(`已安全导出 ${count} 个任务`);
    } catch (error) { notify(String(error), "error"); }
  };
  const importTasks = async () => {
    try {
      const path = await pickPath({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      const destination = await pickPath({ directory: true, multiple: false, title: "选择导入任务的下载目录" });
      if (typeof destination !== "string") return;
      const tasks = await api.importTasks(path, destination);
      notify(`已导入 ${tasks.length} 个任务，均保持暂停`);
    } catch (error) { notify(String(error), "error"); }
  };
  const openLogsDir = async () => {
    try {
      await api.openLogsDir();
    } catch (error) { notify(String(error), "error"); }
  };
  const exportRecentLogs = async () => {
    try {
      const path = await savePath({ defaultPath: "maobu-logs.txt", filters: [{ name: "日志", extensions: ["log", "txt"] }] });
      if (!path) return;
      const count = await api.exportRecentLogs(path);
      notify(`已导出 ${count} 个日志文件（已脱敏）`);
    } catch (error) { notify(String(error), "error"); }
  };
  // Task 27：完整备份与恢复组件。
  const [backupOpen, setBackupOpen] = useState(false);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const set = <K extends keyof AppSettings>(key: K, val: AppSettings[K]) => setDraft((item) => ({ ...item, [key]: val }));
  // Task 22.4：颜色方案变更时同步旧 `theme` 字符串字段，保证后端读 settings.theme 的旧代码仍可用。
  const setColorScheme = (scheme: ColorScheme) => setDraft((item) => ({ ...item, color_scheme: scheme, theme: scheme }));

  const hasSaved = useRef(false);
  const originalSize = useRef<{ width: number; height: number } | null>(null);
  const draftRef = useRef(draft);
  useEffect(() => { draftRef.current = draft; }, [draft]);

  useEffect(() => {
    if (!appWindow) return;
    void Promise.all([
      appWindow.outerSize(),
      appWindow.scaleFactor()
    ]).then(([size, factor]) => {
      originalSize.current = {
        width: Math.round(size.width / factor),
        height: Math.round(size.height / factor)
      };
    });

    return () => {
      const currentDraft = draftRef.current;
      const hasChangedSize = currentDraft.window_width !== value.window_width || currentDraft.window_height !== value.window_height;
      if (!hasSaved.current && hasChangedSize && originalSize.current) {
        void appWindow.setSize(new LogicalSize(originalSize.current.width, originalSize.current.height));
      }
    };
  }, [appWindow, value.window_width, value.window_height]);

  const applyTemporarySize = (w: number, h: number) => {
    if (appWindow) {
      void appWindow.setSize(new LogicalSize(w, h));
    }
  };

  const changeWidth = (val: number | undefined) => {
    set("window_width", val);
    if (val && draft.window_height) {
      applyTemporarySize(val, draft.window_height);
    }
  };

  const changeHeight = (val: number | undefined) => {
    set("window_height", val);
    if (draft.window_width && val) {
      applyTemporarySize(draft.window_width, val);
    }
  };
  const [cacheSizeBytes, setCacheSizeBytes] = useState<number | null>(null);
  const [cacheSizeLoading, setCacheSizeLoading] = useState(false);
  const [cacheCleaning, setCacheCleaning] = useState(false);

  const handleInspectCache = useCallback(() => {
    setCacheSizeLoading(true);
    api.cacheInspect()
      .then((res) => setCacheSizeBytes(res.total_bytes))
      .catch((err) => notify(String(err), "error"))
      .finally(() => setCacheSizeLoading(false));
  }, [notify]);

  const handleClearCache = useCallback(() => {
    setCacheCleaning(true);
    api.cacheClear()
      .then((res) => {
        notify(`清理完成，已释放 ${formatBytes(res.freed_bytes)} 磁盘空间`);
        setCacheSizeBytes(0);
      })
      .catch((err) => notify(String(err), "error"))
      .finally(() => setCacheCleaning(false));
  }, [notify]);

  useEffect(() => {
    if (section === "advanced" && cacheSizeBytes === null) {
      handleInspectCache();
    }
  }, [section, cacheSizeBytes, handleInspectCache]);

  useEffect(() => { let unlisten: (() => void) | undefined; if (section === "browser") void api.pairing().then(setPair); if (section === "media") { void api.toolStatus().then(setTools); void api.subscribeMediaTools(setTools).then((value) => { unlisten = value; }); } return () => unlisten?.(); }, [section]);
  useEffect(() => {
    const applyDraftColorScheme = () => {
      const dark = usesDarkTheme(draft.color_scheme);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
      document.documentElement.dataset.accent = draft.accent_color;
      // Task 22.4：设置页预览时同步 body.light/dark 类。
      document.body.classList.toggle("dark", dark);
      document.body.classList.toggle("light", !dark);
      void applyWindowAppearance(draft.frosted_glass, dark).catch((error) => {
        document.documentElement.dataset.windowStyle = "solid";
        notify(`无法预览磨砂玻璃效果：${String(error)}`, "error");
      });
    };
    applyDraftColorScheme();
    // Task 22.4：System 模式下监听 prefers-color-scheme 变化，便于在设置预览中即时反映。
    if (draft.color_scheme !== "system") return;
    const media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", applyDraftColorScheme);
    return () => media.removeEventListener("change", applyDraftColorScheme);
  }, [draft.color_scheme, draft.accent_color, draft.frosted_glass]);
  // Task 22.2：设置页预览时同步 body.row-compact 类，反映紧凑行高变化。
  useEffect(() => {
    document.body.classList.toggle("row-compact", draft.row_compact);
  }, [draft.row_compact]);
  useEffect(() => {
    const applyDraftScale = () => {
      if (draft.auto_scale_ui) {
        const baseWidth = 1024;
        const scale = window.outerWidth / baseWidth;
        const clampedScale = Math.min(Math.max(scale, 0.75), 2.0);
        document.documentElement.style.zoom = String(clampedScale);
      } else {
        document.documentElement.style.zoom = "";
      }
    };
    applyDraftScale();
    window.addEventListener("resize", applyDraftScale);
    return () => {
      window.removeEventListener("resize", applyDraftScale);
    };
  }, [draft.auto_scale_ui]);
  useEffect(() => {
    return () => {
      const finalSettings = hasSaved.current ? draftRef.current : value;
      setLocale(finalSettings.language || "zh-CN");
      const dark = usesDarkTheme(finalSettings.color_scheme);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
      document.documentElement.dataset.accent = finalSettings.accent_color;
      // Task 22.4：设置页关闭时恢复 body.light/dark 类到最终保存值。
      document.body.classList.toggle("dark", dark);
      document.body.classList.toggle("light", !dark);
      // Task 22.2：恢复 body.row-compact 类到最终保存值。
      document.body.classList.toggle("row-compact", finalSettings.row_compact);
      void applyWindowAppearance(finalSettings.frosted_glass, dark);
      if (finalSettings.auto_scale_ui) {
        const scale = window.outerWidth / 1024;
        const clampedScale = Math.min(Math.max(scale, 0.75), 2.0);
        document.documentElement.style.zoom = String(clampedScale);
      } else {
        document.documentElement.style.zoom = "";
      }
    };
  }, [value]);
  const save = async () => { try { await api.saveSettings(draft); hasSaved.current = true; onChange(draft); notify(t("toasts.settingsSaved")); onClose(); } catch (error) { notify(String(error), "error"); } };
  // Task 33: 设置页导航项按当前 locale 渲染，语言切换时自动刷新。
  const items: Array<[SettingsSection, string, typeof Settings]> = [["general",t("settings.sectionGeneral"),Settings],["download",t("settings.sectionDownload"),Download],["network",t("settings.sectionNetwork"),Network],["browser",t("settings.sectionBrowser"),Globe2],["media",t("settings.sectionMedia"),Video],["rules",t("settings.sectionRules"),ListFilter],["filename-cleanup",t("settings.sectionFilenameCleanup"),Sparkles],["naming-template",t("settings.sectionNamingTemplate"),File],["presets",t("settings.sectionPresets"),Zap],["templates",t("settings.sectionTemplates"),Bookmark],["tags",t("settings.sectionTags"),TagIcon],["credentials",t("settings.sectionCredentials"),ShieldCheck],["appearance",t("settings.sectionAppearance"),SlidersHorizontal],["shortcuts",t("settings.sectionShortcuts"),Keyboard],["advanced",t("settings.sectionAdvanced"),Info],["about",t("settings.sectionAbout"),Info]];
  return <div className="settings-page"><aside className="nav-pane"><div className="brand" data-tauri-drag-region>{t("settings.title")}</div><div className="settings-nav-list">{items.map(([key,label,Icon]) => <button key={key} className={section === key ? "nav-item active" : "nav-item"} onClick={() => setSection(key)}><Icon size={15} /><span>{label}</span></button>)}</div><div className="nav-footer"><button className="nav-settings" onClick={onClose} title={t("settings.returnHome")}><ArrowLeft size={15} /><span>{t("settings.returnHome")}</span></button><div className="nav-status" style={{ cursor: "default" }}><i className={isDesktop() ? "status-dot online" : "status-dot offline"} /><span>{t("nav.speedFormat", { speed: `${formatBytes(totalSpeed)}/s`, count: activeCount })}</span></div></div></aside><main className="settings-body" data-tauri-drag-region><div className="settings-title" data-tauri-drag-region><h1 data-tauri-drag-region>{items.find(([key]) => key === section)?.[1]}</h1></div><div className="settings-content">
    {section === "general" && <>
      <SettingsGroup title={t("settings.groupLanguage")}>
        <div className="settings-group-content">
          <SettingRow label={t("settings.languageLabel")}>
            <Select
              value={draft.language || "zh-CN"}
              onChange={(nextVal) => {
                const next = String(nextVal);
                set("language", next);
                setLocale(next);
              }}
              options={[
                { value: "zh-CN", label: t("settings.languageZhCN") },
                { value: "en", label: t("settings.languageEn") },
              ]}
              ariaLabel={t("settings.languageLabel")}
            />
          </SettingRow>
        </div>
        <p className="settings-note">{t("settings.languageHint")}</p>
      </SettingsGroup>
      <SettingsGroup title={t("settings.groupAppBehavior")}><div className="settings-group-content"><Toggle label={t("settings.autoStart")} checked={draft.auto_start} onChange={(v) => set("auto_start", v)} /><Toggle label={t("settings.startMinimized")} checked={draft.start_minimized} onChange={(v) => set("start_minimized", v)} /><Toggle label={t("settings.minimizeToTray")} checked={draft.minimize_to_tray} onChange={(v) => set("minimize_to_tray", v)} /><Toggle label={t("settings.closeToTray")} checked={draft.close_to_tray} onChange={(v) => set("close_to_tray", v)} /><Toggle label={t("settings.notifyComplete")} checked={draft.notifications} onChange={(v) => set("notifications", v)} /><Toggle label={t("settings.monitorClipboard")} checked={draft.clipboard_monitor} onChange={(v) => set("clipboard_monitor", v)} /></div></SettingsGroup>
      <SettingsGroup title={t("settings.groupNotifications")}>
        <div className="settings-group-content">
          <Toggle label={t("settings.notifyOnComplete")} checked={draft.notify_on_complete} onChange={(v) => set("notify_on_complete", v)} />
          <Toggle label={t("settings.notifyOnFailure")} checked={draft.notify_on_failure} onChange={(v) => set("notify_on_failure", v)} />
          <Toggle label={t("settings.notifySoundEnabled")} checked={draft.notify_sound_enabled} onChange={(v) => set("notify_sound_enabled", v)} />
          <Toggle label={t("settings.notifyFailureSoundEnabled")} checked={draft.notify_failure_sound_enabled} onChange={(v) => set("notify_failure_sound_enabled", v)} />
        </div>
        <p className="settings-note">{t("settings.notifySoundDesc")}</p>
      </SettingsGroup>
      <SettingsGroup title={t("settings.groupHistoryArchive")}>
        <div className="settings-group-content">
          <SettingRow label={t("settings.archiveDaysLabel")}><input type="number" min="0" max="3650" value={draft.archive_days} onChange={(e) => set("archive_days", Math.max(0, +e.target.value || 0))} /></SettingRow>
          <SettingRow label={t("settings.archiveThresholdLabel")}><input type="number" min="0" max="100000" value={draft.archive_threshold} onChange={(e) => set("archive_threshold", Math.max(0, +e.target.value || 0))} /></SettingRow>
        </div>
        <p className="settings-note">{t("settings.archiveDesc")}</p>
      </SettingsGroup>
    </>}
    {section === "download" && <SettingsGroup title="保存与性能">
      <div className="settings-group-content">
        <SettingRow label="默认下载目录"><input value={draft.download_dir} onChange={(e) => set("download_dir", e.target.value)} /></SettingRow>
        <SettingRow label="文件重名"><div className="fluent-segmented-control settings-segmented"><button type="button" className={draft.default_collision_policy === "rename" ? "active" : ""} onClick={() => set("default_collision_policy", "rename")}>自动重命名</button><button type="button" className={draft.default_collision_policy === "overwrite" ? "active" : ""} onClick={() => set("default_collision_policy", "overwrite")}>覆盖</button><button type="button" className={draft.default_collision_policy === "skip" ? "active" : ""} onClick={() => set("default_collision_policy", "skip")}>跳过</button></div></SettingRow>
        <SettingRow label="默认完成动作"><div className="setting-completion-action"><CompletionActionEditor value={draft.default_completion_action} onChange={(a) => set("default_completion_action", a)} /></div></SettingRow>
        <Toggle label="低内存模式（1 个任务、每任务最多 2 路连接）" checked={draft.low_memory_mode} onChange={(v) => set("low_memory_mode", v)} />
        <SettingRow label="同时下载任务"><input type="number" min="1" max="16" value={draft.concurrent_downloads} onChange={(e) => set("concurrent_downloads", +e.target.value)} /></SettingRow>
        <SettingRow label={`每任务连接数 (${draft.connections_per_download} 路)`}><div className="settings-slider-wrapper"><input type="range" min="0" max="5" step="1" value={[1, 2, 4, 8, 16, 32].indexOf(draft.connections_per_download)} onChange={(e) => { const values = [1, 2, 4, 8, 16, 32]; set("connections_per_download", values[+e.target.value]); }} className="fluent-slider" /><div className="slider-ticks"><span>1</span><span>2</span><span>4</span><span>8</span><span>16</span><span>32</span></div></div></SettingRow>
        <SettingRow label="全局限速（KB/s）"><input type="number" min="0" value={draft.speed_limit_kbps} onChange={(e) => set("speed_limit_kbps", +e.target.value)} /></SettingRow>
        <Toggle label="完成后计算 SHA-256" checked={draft.verify_after_download} onChange={(v) => set("verify_after_download", v)} />
      </div>
      <p className="settings-note">开启低内存模式后使用更小的合并缓冲区和连接池；不会改写并发偏好，关闭后自动恢复。</p>
    </SettingsGroup>}
    {section === "network" && <>
      <SettingsGroup title="代理设置">
        <div className="settings-group-content">
          <SettingRow label="代理模式">
            <Select
              value={draft.proxy_mode}
              onChange={(val: any) => set("proxy_mode", val as AppSettings["proxy_mode"])}
              options={[
                { value: "system", label: "跟随系统" },
                { value: "none", label: "不使用代理" },
                { value: "manual", label: "手动代理" },
              ]}
              ariaLabel="代理模式"
            />
          </SettingRow>
          {draft.proxy_mode === "manual" && <>
            <SettingRow label="代理地址"><input value={draft.proxy_url} onChange={(e) => set("proxy_url", e.target.value)} placeholder="http://host:port 或 socks5://host:port" /></SettingRow>
            <SettingRow label="用户名"><input value={draft.proxy_username} onChange={(e) => set("proxy_username", e.target.value)} placeholder="匿名代理可留空" /></SettingRow>
            <SettingRow label="密码"><input type="password" value={draft.proxy_password} onChange={(e) => set("proxy_password", e.target.value)} placeholder="无认证可留空" /></SettingRow>
            <SettingRow label="测试连通性">
              <ProxyTestButton
                proxyUrl={draft.proxy_url}
                auth={draft.proxy_username || draft.proxy_password ? { username: draft.proxy_username, password: draft.proxy_password } : null}
                notify={notify}
              />
            </SettingRow>
          </>}
          <SettingRow label="PAC 脚本路径（可选）"><input value={draft.pac_script_path ?? ""} onChange={(e) => set("pac_script_path", e.target.value || null)} placeholder="C:\path\to\proxy.pac，留空表示不使用" /></SettingRow>
        </div>
        <p className="settings-note">代理密码使用 Windows DPAPI 加密存储。可在任务详情面板中为单个任务配置覆盖代理。</p>
      </SettingsGroup>
      <SettingsGroup title="重试策略">
        <div className="retry-policy-grid">
          <RetryPolicyEditor value={draft.default_retry_policy} onChange={(p) => set("default_retry_policy", p)} compact />
        </div>
        <p className="settings-note">在此设置默认重试与超时规则。单个任务可在其详情面板中独立配置进行覆盖。</p>
      </SettingsGroup>
      <SettingsGroup title="网络感知">
        <div className="settings-group-content">
          <Toggle label="计量网络自动暂停" checked={draft.metered_auto_pause} onChange={(v) => set("metered_auto_pause", v)} />
          <SettingRow label="立即检查当前网络">
            <MeteredCheckButton notify={notify} />
          </SettingRow>
        </div>
        <p className="settings-note">每 60 秒检测一次网络计费状态。检测到计量网络（按量计费）时将自动暂停下载以节省流量，用户手动恢复后不再自动暂停。</p>
      </SettingsGroup>
    </>}
    {section === "browser" && <><SettingsGroup title="下载接管"><div className="settings-group-content"><Toggle label="允许浏览器扩展接管下载" checked={draft.intercept_browser_downloads} onChange={(v) => set("intercept_browser_downloads", v)} /><SettingRow label="最小文件大小（MB）"><input type="number" min="0" value={draft.min_file_size_mb} onChange={(e) => set("min_file_size_mb", +e.target.value)} /></SettingRow></div></SettingsGroup><SettingsGroup title="安全配对">{pair ? <div className="pair-card"><p>在扩展中输入一次性配对码（10 分钟有效）</p><div className="pair-code-wrapper"><code>{pair.code}</code><button className="copy-code-btn" onClick={() => { void navigator.clipboard.writeText(pair.code); notify("配对码已复制到剪贴板"); }} title="复制配对码"><Copy size={13} /><span>复制</span></button></div>{pair.paired_extension && <p>已配对：{pair.paired_extension.slice(0, 16)}…</p>}<div className="maintenance"><button onClick={() => void api.rotatePairing().then(setPair)}>更换配对码</button>{pair.paired_extension && <button onClick={() => void api.revokePairing().then(() => api.pairing().then(setPair))}>撤销配对</button>}</div></div> : <LoaderCircle className="spin" />}</SettingsGroup></>}
    {section === "media" && <SettingsGroup title="媒体组件"><p className="settings-note">按“自定义路径 → 应用安装 → Windows PATH”顺序查找组件。外部组件只会被引用，猫步下载器不会复制、更新或删除它们。</p>{tools ? <MediaToolsCard status={tools} onStatus={setTools} /> : <LoaderCircle className="spin" />}<MediaPathSettings value={draft} onChange={(patch) => setDraft((current) => ({ ...current, ...patch }))} /><MediaToolsUpdateRow tools={tools} onStatus={setTools} /></SettingsGroup>}
    {section === "rules" && <CategoryRulesPanel notify={notify} />}
    {section === "filename-cleanup" && <FilenameCleanupPanel notify={notify} />}
    {section === "naming-template" && <PlatformNamingTemplatePanel notify={notify} />}
    {section === "presets" && <PresetsPanel notify={notify} />}
    {section === "templates" && <TaskTemplatesPanel notify={notify} />}
    {section === "tags" && <TagManagementPanel notify={notify} />}
    {section === "credentials" && <MediaCredentialsPanel notify={notify} />}
    {section === "appearance" && <>
      <SettingsGroup title="主题与紧凑度">
        <div className="settings-group-content">
          <SettingRow label="颜色方案"><div className="fluent-segmented-control settings-segmented"><button type="button" className={draft.color_scheme === "system" ? "active" : ""} onClick={() => setColorScheme("system")}>跟随系统</button><button type="button" className={draft.color_scheme === "light" ? "active" : ""} onClick={() => setColorScheme("light")}>浅色</button><button type="button" className={draft.color_scheme === "dark" ? "active" : ""} onClick={() => setColorScheme("dark")}>深色</button></div></SettingRow>
          <SettingRow label="行高"><div className="fluent-segmented-control settings-segmented"><button type="button" className={!draft.row_compact ? "active" : ""} onClick={() => set("row_compact", false)}>标准 (36px)</button><button type="button" className={draft.row_compact ? "active" : ""} onClick={() => set("row_compact", true)}>紧凑 (32px)</button></div></SettingRow>
          <Toggle label="详情栏默认折叠（切换任务时）" checked={draft.detail_default_collapsed} onChange={(v) => set("detail_default_collapsed", v)} />
          <SettingRow label="强调色">
            <Select
              value={draft.accent_color}
              onChange={(val: any) => set("accent_color", val as AppSettings["accent_color"])}
              options={[
                { value: "system", label: "跟随 Windows" },
                { value: "blue", label: "猫步蓝" },
                { value: "cyan", label: "青色" },
                { value: "green", label: "绿色" },
                { value: "purple", label: "紫色" },
                { value: "orange", label: "橙色" },
              ]}
              ariaLabel="强调色"
            />
          </SettingRow>
          <Toggle label="磨砂玻璃" checked={draft.frosted_glass} onChange={(v) => set("frosted_glass", v)} />
        </div>
        <p className="settings-note">在此设置颜色方案、行高大小、强调色以及详情栏折叠与磨砂玻璃等外观偏好。</p>
      </SettingsGroup>
      <SettingsGroup title="窗口大小">
        <div className="settings-group-content">
          <SettingRow label="窗口大小">
            <div className="window-size-setting-row">
              <input type="number" placeholder="宽度 (如 800)" value={draft.window_width || ""} onChange={(e) => changeWidth(e.target.value ? +e.target.value : undefined)} className="window-size-input" />
              <span>×</span>
              <input type="number" placeholder="高度 (如 600)" value={draft.window_height || ""} onChange={(e) => changeHeight(e.target.value ? +e.target.value : undefined)} className="window-size-input" />
              <Select
                value={draft.window_width && draft.window_height ? `${draft.window_width}x${draft.window_height}` : ""}
                onChange={(val: any) => {
                  if (!val) return;
                  const [w, h] = String(val).split("x").map(Number);
                  set("window_width", w);
                  set("window_height", h);
                  applyTemporarySize(w, h);
                }}
                options={[
                  { value: "", label: "选择常用预设..." },
                  { value: "800x600", label: "800 × 600 (迷你紧凑)" },
                  { value: "960x640", label: "960 × 640 (精致比例)" },
                  { value: "1024x720", label: "1024 × 720 (默认标准)" },
                  { value: "1120x760", label: "1120 × 760 (舒适格局)" },
                  { value: "1280x800", label: "1280 × 800 (高效宽屏)" },
                  { value: "1440x900", label: "1440 × 900 (专业超宽)" },
                ]}
                ariaLabel="预设窗口大小"
                className="window-size-preset-select"
              />
            </div>
          </SettingRow>
          <Toggle label="自适应缩放" checked={draft.auto_scale_ui || false} onChange={(v) => set("auto_scale_ui", v)} />
        </div>
        <p className="settings-note">磨砂玻璃使用 Windows 10/11 原生 Acrylic 材质；自适应缩放根据窗口宽度自动放大 UI。</p>
      </SettingsGroup>
    </>}
    {section === "shortcuts" && (
      <SettingsGroup title={t("shortcuts.title")}>
        <ShortcutSettingsSection
          value={draft.shortcut_keys || DEFAULT_SHORTCUTS}
          onChange={(val) => set("shortcut_keys", val)}
          notify={notify}
        />
      </SettingsGroup>
    )}
    {section === "advanced" && <><SettingsGroup title="任务迁移"><div className="maintenance"><button onClick={() => void exportTasks()}>导出任务 JSON</button><button onClick={() => void importTasks()}>导入任务 JSON</button></div><p className="settings-note">导出文件不含请求头、凭据和下载进度；导入任务将统一暂停并保存到指定目录。</p></SettingsGroup><SettingsGroup title="备份与恢复"><div className="maintenance"><button onClick={() => setBackupOpen(true)}>创建完整备份</button><button onClick={() => setRestoreOpen(true)}>从备份恢复</button></div><p className="settings-note">备份包含设置、规则与任务列表。勾选“包含认证信息”后将加密保护。恢复时已存在任务会自动跳过。</p></SettingsGroup><SettingsGroup title="日志"><div className="maintenance"><button onClick={() => void openLogsDir()}>打开日志目录</button><button onClick={() => void exportRecentLogs()}>导出最近 24 小时日志</button></div><p className="settings-note">日志滚动保留 7 天，敏感凭证已自动脱敏。出于安全考虑，日志目录路径不对前端公开。</p></SettingsGroup><SettingsGroup title="维护">
  <div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
    <div className="maintenance">
      <button onClick={() => void api.clearHistory(false).then(() => notify("已清理取消的任务"))}>清理取消任务</button>
      <button onClick={() => void api.clearHistory(true).then(() => notify("下载历史已清理"))}>清理完成和取消任务</button>
    </div>
    <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", paddingTop: "10px", borderTop: "1px solid var(--border)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: "8px", fontSize: "12px", color: "var(--text)" }}>
        <span>软件缓存：</span>
        <strong style={{ color: "var(--primary)" }}>
          {cacheSizeLoading ? "计算中..." : cacheSizeBytes !== null ? formatBytes(cacheSizeBytes) : "—"}
        </strong>
      </div>
      <div className="maintenance" style={{ marginTop: 0 }}>
        <button onClick={() => void handleInspectCache()} disabled={cacheSizeLoading || cacheCleaning}>
          检查缓存
        </button>
        <button onClick={() => void handleClearCache()} disabled={cacheCleaning || cacheSizeBytes === 0 || cacheSizeBytes === null}>
          {cacheCleaning ? "清理中..." : "清理软件缓存"}
        </button>
      </div>
    </div>
  </div>
</SettingsGroup></>}
    <BackupRestoreModal notify={notify} backupOpen={backupOpen} setBackupOpen={setBackupOpen} restoreOpen={restoreOpen} setRestoreOpen={setRestoreOpen} />
    {section === "about" && (
      <SettingsGroup title={t("settings.groupAboutMaobu")}>
        <div style={{ display: "flex", flexDirection: "column", gap: "16px", padding: "10px 0" }}>
          <div style={{ display: "flex", alignItems: "center", gap: "16px" }}>
            <div style={{ width: "64px", height: "64px", flexShrink: 0 }}>
              <CatDownloadMark />
            </div>
            <div>
              <h2 style={{ margin: 0, fontSize: "16px", fontWeight: 700, color: "var(--text)" }}>猫步下载器 (Maobu Fetch)</h2>
              <p style={{ margin: "4px 0 0", fontSize: "11px", color: "var(--muted)" }}>版本 0.6.0</p>
            </div>
          </div>

          {appInfo?.portable_mode && (
            <div
              role="status"
              style={{
                display: "flex",
                alignItems: "flex-start",
                gap: "8px",
                padding: "10px 12px",
                borderRadius: "6px",
                border: "1px solid var(--accent)",
                background: "rgba(59,130,246,0.08)",
                color: "var(--text)",
                fontSize: "11px",
                lineHeight: 1.5,
              }}
            >
              <ShieldCheck size={14} color="var(--accent)" style={{ flexShrink: 0, marginTop: "1px" }} />
              <div>
                <strong style={{ color: "var(--accent)" }}>便携模式已启用</strong>
                <div style={{ marginTop: "2px", color: "var(--muted)" }}>
                  数据存储于 EXE 同目录的 <code style={{ fontSize: "11px", padding: "1px 4px", borderRadius: "3px", background: "var(--bg-alt, rgba(0,0,0,0.04))", border: "1px solid var(--border)" }}>data/</code> 文件夹，不写入系统 <code style={{ fontSize: "11px", padding: "1px 4px", borderRadius: "3px", background: "var(--bg-alt, rgba(0,0,0,0.04))", border: "1px solid var(--border)" }}>%APPDATA%</code>。可将整个程序目录复制到任意位置或设备使用。
                </div>
              </div>
            </div>
          )}

          <div style={{ borderTop: "1px solid var(--border)", paddingTop: "14px", display: "flex", alignItems: "center", gap: "12px" }}>
            <span style={{ fontSize: "12px", fontWeight: 600, color: "var(--muted)" }}>作者 / 开发团队</span>
            <div style={{ display: "inline-flex", alignItems: "center", gap: "6px" }}>
              <span style={{ fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>猫步可爱</span>
              <span style={{ fontSize: "11px", color: "var(--subtle)" }}>(maobukeai)</span>
            </div>
          </div>

          <div style={{ borderTop: "1px solid var(--border)", paddingTop: "14px" }}>
            <h3 style={{ margin: "0 0 6px", fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>软件技术架构</h3>
            <p style={{ margin: 0, fontSize: "11px", color: "var(--muted)", lineHeight: 1.6 }}>
              猫步下载器采用现代化、高性能且低开销的桌面端产品架构：
            </p>
            <ul style={{ margin: "6px 0 0", paddingLeft: "16px", fontSize: "11px", color: "var(--muted)", lineHeight: 1.6 }}>
              <li><strong>前端展示层 (Frontend)</strong>: 基于 React 19 + TypeScript + Vite，配合极致轻量的 Vanilla CSS 实现，无第三方重型组件库，界面精细紧凑。</li>
              <li><strong>桌面后端层 (Backend)</strong>: 基于 Rust 核心与 Tauri v2 框架，保证极高的执行性能与近乎为零的待机内存开销。</li>
              <li><strong>数据持久层 (Database)</strong>: 使用嵌入式 SQLite 关系型数据库，安全快速地持久化下载任务队列与用户偏好。</li>
              <li><strong>多线程下载引擎 (Engine)</strong>: 高并发 HTTP Range 切片下载，支持动态断点续传与速度限制，按需支持 yt-dlp 与 FFmpeg 媒体源分析。</li>
            </ul>
          </div>

          <div style={{ borderTop: "1px solid var(--border)", paddingTop: "14px", display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: "16px" }}>
            {/* 1. 应用更新检查 */}
            <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
              <h3 style={{ margin: "0", fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>应用更新检查</h3>
              <p style={{ margin: 0, fontSize: "11px", color: "var(--muted)", lineHeight: 1.4 }}>
                查询 Releases 最新版本并手动校验。
              </p>
              <div style={{ display: "flex", alignItems: "center", gap: "8px", flexWrap: "wrap", marginTop: "auto", paddingTop: "4px" }}>
                <button
                  className="input-button"
                  disabled={updateChecking}
                  onClick={() => void checkAppUpdate()}
                  style={{ display: "inline-flex", alignItems: "center", gap: "6px", height: "28px", padding: "0 12px", fontSize: "11px", fontWeight: 500, cursor: updateChecking ? "default" : "pointer", borderRadius: "6px", border: "1px solid var(--border)", background: "var(--accent)", color: "white" }}
                >
                  {updateChecking ? <LoaderCircle size={12} className="spin" /> : <RefreshCw size={12} />}
                  {updateChecking ? "检查中…" : "检查更新"}
                </button>
                <span style={{ fontSize: "11px", color: "var(--muted)" }}>v0.6.0</span>
              </div>
              {updateResult && !updateResult.error && (
                <div style={{ marginTop: "6px", fontSize: "11px", lineHeight: 1.5, color: "var(--muted)", padding: "8px 10px", background: "var(--bg-alt, rgba(0,0,0,0.03))", borderRadius: "6px", border: "1px solid var(--border)" }}>
                  {updateResult.has_update && updateResult.latest ? (
                    <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
                      <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
                        <AlertCircle size={12} color="var(--accent)" />
                        <strong style={{ color: "var(--text)" }}>发现新版本 v{updateResult.latest.version}</strong>
                        {updateResult.latest.release_date && <span style={{ color: "var(--muted)" }}>· {updateResult.latest.release_date}</span>}
                      </div>
                      {updateResult.latest.release_notes && (
                        <div style={{ maxHeight: "120px", overflowY: "auto", whiteSpace: "pre-wrap", fontSize: "11px", color: "var(--muted)", borderTop: "1px solid var(--border)", paddingTop: "6px" }}>
                          {updateResult.latest.release_notes}
                        </div>
                      )}
                      {updateResult.latest.sha256 && (
                        <div style={{ fontSize: "10px", color: "var(--muted)", wordBreak: "break-all" }}>SHA-256: {updateResult.latest.sha256}</div>
                      )}
                      <div>
                        <button
                          className="input-button"
                          onClick={() => void openUrl(updateResult.latest?.download_url || "https://github.com/maobukeai/maobu-fetch/releases").catch((err) => notify(String(err), "error"))}
                          style={{ display: "inline-flex", alignItems: "center", gap: "6px", height: "26px", padding: "0 12px", fontSize: "11px", fontWeight: 500, cursor: "pointer", borderRadius: "6px", border: "1px solid var(--accent)", background: "transparent", color: "var(--accent)" }}
                        >
                          <ExternalLink size={11} />
                          前往下载页
                        </button>
                      </div>
                    </div>
                  ) : (
                    <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
                      <Check size={12} color="#22c55e" />
                      <span>已是最新版{updateResult.latest ? ` (v${updateResult.latest.version})` : ""}</span>
                    </div>
                  )}
                </div>
              )}
              {updateResult?.error && (
                <div style={{ marginTop: "6px", fontSize: "11px", color: "var(--danger, #ef4444)", padding: "8px 10px", background: "rgba(239,68,68,0.08)", borderRadius: "6px", border: "1px solid rgba(239,68,68,0.2)", display: "flex", alignItems: "center", gap: "6px" }}>
                  <AlertCircle size={12} />
                  <span>检查失败：{updateResult.error}</span>
                </div>
              )}
            </div>

            {/* 2. 扩展版本兼容性 */}
            <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
              <h3 style={{ margin: "0", fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>扩展版本兼容性</h3>
              <p style={{ margin: 0, fontSize: "11px", color: "var(--muted)", lineHeight: 1.4 }}>
                输入扩展版本号，验证与桌面端兼容性。
              </p>
              <div style={{ display: "flex", alignItems: "center", gap: "6px", flexWrap: "wrap", marginTop: "auto", paddingTop: "4px" }}>
                <input
                  value={extVersion}
                  onChange={(e) => setExtVersion(e.target.value)}
                  placeholder="如 0.6.0"
                  style={{ height: "28px", padding: "0 8px", fontSize: "11px", borderRadius: "6px", border: "1px solid var(--border)", background: "var(--bg)", color: "var(--text)", width: "85px" }}
                />
                <button
                  className="input-button"
                  disabled={extChecking}
                  onClick={() => void checkExtCompat()}
                  style={{ display: "inline-flex", alignItems: "center", gap: "6px", height: "28px", padding: "0 10px", fontSize: "11px", fontWeight: 500, cursor: extChecking ? "default" : "pointer", borderRadius: "6px", border: "1px solid var(--border)", background: "var(--bg)", color: "var(--text)" }}
                >
                  {extChecking ? <LoaderCircle size={11} className="spin" /> : <ShieldCheck size={11} />}
                  {extChecking ? "检查中…" : "检查兼容性"}
                </button>
              </div>
              {extResult && (
                <div style={{ marginTop: "6px", fontSize: "11px", lineHeight: 1.5, padding: "8px 10px", borderRadius: "6px", border: extResult.compatible ? "1px solid rgba(34,197,94,0.3)" : "1px solid rgba(239,68,68,0.3)", background: extResult.compatible ? "rgba(34,197,94,0.08)" : "rgba(239,68,68,0.08)", color: "var(--muted)" }}>
                  <div style={{ display: "flex", alignItems: "center", gap: "6px", marginBottom: "4px" }}>
                    {extResult.compatible ? <Check size={12} color="#22c55e" /> : <AlertCircle size={12} color="#ef4444" />}
                    <strong style={{ color: "var(--text)" }}>
                      {extResult.compatible ? "兼容" : "不兼容"}
                    </strong>
                    <span style={{ color: "var(--muted)" }}>· 扩展 v{extResult.extension_version} / 桌面 v{extResult.app_version}</span>
                  </div>
                  {extResult.message && <div>{extResult.message}</div>}
                </div>
              )}
            </div>

            {/* 3. 开源项目主页 */}
            <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
              <h3 style={{ margin: "0", fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>开源项目主页</h3>
              <p style={{ margin: 0, fontSize: "11px", color: "var(--muted)", lineHeight: 1.4 }}>
                访问 GitHub 仓库获取最新源码与参与贡献。
              </p>
              <div style={{ display: "flex", alignItems: "center", gap: "8px", marginTop: "auto", paddingTop: "4px" }}>
                <button
                  className="input-button"
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: "6px",
                    height: "28px",
                    padding: "0 12px",
                    fontSize: "11px",
                    fontWeight: 500,
                    cursor: "pointer",
                    borderRadius: "6px",
                    border: "1px solid var(--border)",
                    background: "var(--accent)",
                    color: "white"
                  }}
                  onClick={async () => {
                    try {
                      await openUrl("https://github.com/maobukeai/maobu-fetch");
                    } catch (err) {
                      notify(String(err), "error");
                    }
                  }}
                >
                  <ExternalLink size={12} />
                  访问 GitHub
                </button>
              </div>
            </div>
          </div>

          {/* Task 44: 平台兼容性矩阵子区域。
              展示各媒体平台的支持级别（已验证/实验性/不支持），帮助用户预期下载行为。
              内置 6 条记录（YouTube/哔哩哔哩=Verified，抖音/TikTok/Twitter/微博=Experimental）。
              徽章同时使用颜色和文字标识，不依赖单一颜色（AGENTS.md §4 无障碍）。 */}
          <div style={{ borderTop: "1px solid var(--border)", paddingTop: "14px", display: "flex", flexDirection: "column", gap: "10px" }}>
            <h3 style={{ margin: "0", fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>平台兼容性</h3>
            <p style={{ margin: 0, fontSize: "11px", color: "var(--muted)", lineHeight: 1.5 }}>
              以下是各媒体平台的支持级别，新建任务时将根据匹配到的平台展示对应状态徽章。
            </p>
            {platformCompatList.length === 0 ? (
              <div style={{ fontSize: "11px", color: "var(--muted)", padding: "8px 10px", background: "var(--bg-alt, rgba(0,0,0,0.03))", borderRadius: "6px", border: "1px solid var(--border)" }}>
                暂无平台兼容性数据
              </div>
            ) : (
              <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))", gap: "8px" }}>
                {platformCompatList.map((item) => {
                  const platformLabel = (() => {
                    switch (item.platform) {
                      case "douyin": return "抖音";
                      case "tiktok": return "TikTok";
                      case "twitter": return "Twitter/X";
                      case "youtube": return "YouTube";
                      case "bilibili": return "哔哩哔哩";
                      case "weibo": return "微博";
                      default: return item.platform;
                    }
                  })();
                  return (
                    <div
                      key={item.platform}
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        gap: "4px",
                        padding: "8px 10px",
                        borderRadius: "6px",
                        border: "1px solid var(--border)",
                        background: "var(--bg-alt, rgba(0,0,0,0.02))",
                      }}
                    >
                      <div style={{ display: "flex", alignItems: "center", gap: "6px", flexWrap: "wrap" }}>
                        <Globe2 size={12} color="var(--muted)" />
                        <strong style={{ fontSize: "11px", color: "var(--text)" }}>{platformLabel}</strong>
                        <span
                          title={supportLevelLabel(item.level)}
                          style={{
                            marginLeft: "auto",
                            padding: "1px 6px",
                            borderRadius: 4,
                            fontSize: 10,
                            color: "#fff",
                            backgroundColor: supportLevelColor(item.level),
                            border: `1px solid ${supportLevelColor(item.level)}`,
                          }}
                        >
                          {supportLevelLabel(item.level)}
                        </span>
                      </div>
                      {item.notes && (
                        <p style={{ margin: 0, fontSize: "10px", color: "var(--muted)", lineHeight: 1.4 }}>
                          {item.notes}
                        </p>
                      )}
                      {item.known_issues && item.known_issues.length > 0 && (
                        <ul style={{ margin: "2px 0 0", paddingLeft: "14px", fontSize: "10px", color: "var(--muted)", lineHeight: 1.4 }}>
                          {item.known_issues.map((issue, idx) => (
                            <li key={idx}>{issue}</li>
                          ))}
                        </ul>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </SettingsGroup>
    )}
    <div className="dialog-actions settings-actions"><button onClick={onClose}>{t("common.cancel")}</button><button className="primary" onClick={() => void save()}>{t("settings.saveSettings")}</button></div>
  </div></main></div>;
}

function SettingsGroup({ title, children }: { title: string; children: ReactNode }) { return <section className="settings-group"><h2>{title}</h2><div>{children}</div></section>; }
function SettingRow({ label, children }: { label: string; children: ReactNode }) { return <label className="setting-row"><div><strong>{label}</strong></div>{children}</label>; }
function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) { return <label className="setting-row"><div><strong>{label}</strong></div><input className="toggle" type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} /></label>; }
/** Task 32：计量网络立即检查按钮。调用 `network_check_metered` 命令并展示中文结果。 */
function MeteredCheckButton({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [checking, setChecking] = useState(false);
  const onClick = async () => {
    if (checking) return;
    setChecking(true);
    try {
      const metered = await api.networkCheckMetered();
      notify(metered ? "当前为计量网络（按量计费）" : "当前不是计量网络", metered ? "error" : "ok");
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setChecking(false);
    }
  };
  return (
    <button
      type="button"
      className="secondary-btn"
      disabled={checking}
      onClick={() => void onClick()}
    >
      {checking ? <LoaderCircle size={11} className="spin" /> : <Network size={11} />}
      <span>{checking ? "检查中…" : "立即检查"}</span>
    </button>
  );
}

/**
 * Task 31：代理测试按钮。
 *
 * 调用 `api.proxyTest` 通过指定代理 URL 请求 ipify，返回出口 IP 与延迟。
 * 成功时显示"出口 IP: x.x.x.x · 延迟 Nms"；失败时显示脱敏错误。
 * `auth.password` 应为前端输入的明文；此命令不读取数据库。
 */
function ProxyTestButton({ proxyUrl, auth, notify, disabled }: { proxyUrl: string; auth: ProxyAuth | null; notify: (text: string, kind?: "ok" | "error") => void; disabled?: boolean }) {
  const [testing, setTesting] = useState(false);
  const onClick = async () => {
    if (testing) return;
    if (!proxyUrl.trim()) {
      notify("请先填写代理地址", "error");
      return;
    }
    setTesting(true);
    try {
      const result = await api.proxyTest(proxyUrl, auth);
      if (result.success) {
        const ip = result.exit_ip ?? "未知";
        notify(`代理可用 · 出口 IP: ${ip} · 延迟 ${result.latency_ms}ms`, "ok");
      } else {
        notify(result.error ?? "代理测试失败", "error");
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };
  return (
    <button
      type="button"
      className="proxy-test-btn"
      disabled={testing || disabled}
      onClick={() => void onClick()}
    >
      {testing ? <LoaderCircle size={11} className="spin" /> : <Globe2 size={11} />}
      {testing ? "测试中…" : "测试代理"}
    </button>
  );
}
function Field({ label, children, className }: { label: string; children: ReactNode; className?: string }) { return <label className={`form-field ${className || ""}`.trim()}><span>{label}</span>{children}</label>; }
function MediaPathSettings({ value, onChange }: { value: AppSettings; onChange: (patch: Partial<AppSettings>) => void }) {
  const [detecting, setDetecting] = useState(false);
  const [detectionMessage, setDetectionMessage] = useState("");
  const chooseYtDlp = async () => {
    const selected = await pickPath({ multiple: false, filters: [{ name: "yt-dlp", extensions: ["exe"] }] });
    if (typeof selected === "string") onChange({ yt_dlp_path: selected });
  };
  const chooseFfmpeg = async () => {
    const selected = await pickPath({ multiple: false, filters: [{ name: "FFmpeg", extensions: ["exe"] }] });
    if (typeof selected !== "string") return;
    onChange({ ffmpeg_path: selected, ffprobe_path: selected.replace(/[^\\/]+$/, "ffprobe.exe") });
  };
  const detectSystemTools = async () => {
    setDetecting(true);
    setDetectionMessage("");
    try {
      const detected = await api.detectSystemMediaTools();
      const hasYtDlp = Boolean(detected.yt_dlp_path);
      const hasFfmpegPair = Boolean(detected.ffmpeg_path && detected.ffprobe_path);
      const patch: Partial<AppSettings> = {};
      if (detected.yt_dlp_path) patch.yt_dlp_path = detected.yt_dlp_path;
      if (hasFfmpegPair) {
        patch.ffmpeg_path = detected.ffmpeg_path;
        patch.ffprobe_path = detected.ffprobe_path;
      }
      if (Object.keys(patch).length) onChange(patch);
      if (hasYtDlp && hasFfmpegPair) setDetectionMessage("已检测到 yt-dlp、FFmpeg 和 FFprobe，路径已填入下方");
      else if (hasYtDlp) setDetectionMessage("已检测到 yt-dlp 并填入路径；未找到完整的 FFmpeg 与 FFprobe");
      else if (hasFfmpegPair) setDetectionMessage("已检测到 FFmpeg 与 FFprobe 并填入路径；未找到 yt-dlp");
      else if (detected.ffmpeg_path || detected.ffprobe_path) setDetectionMessage("只找到部分 FFmpeg 组件，需要同时存在 ffmpeg.exe 和 ffprobe.exe");
      else setDetectionMessage("未在 PATH 或常见独立安装目录中找到媒体组件，可选择本地文件或按需下载");
    } catch (error) {
      setDetectionMessage(`检测失败：${String(error)}`);
    } finally {
      setDetecting(false);
    }
  };
  return <div className="settings-group-content media-path-settings">
    <div className="media-detect-row">
      <div><strong>自动使用系统已有组件</strong><span className="media-detect-desc">扫描系统环境并自动填入对应路径</span></div>
      <button className="input-button" disabled={detecting} onClick={() => void detectSystemTools()}>{detecting ? "检测中…" : "自动检测"}</button>
    </div>
    {detectionMessage && <p className="media-detect-result" role="status">{detectionMessage}</p>}
    <SettingRow label="自定义 yt-dlp.exe"><div className="input-group"><input value={value.yt_dlp_path} onChange={(event) => onChange({ yt_dlp_path: event.target.value })} placeholder="留空则自动检测" /><button className="input-button" onClick={() => void chooseYtDlp()}>选择文件</button>{value.yt_dlp_path && <button className="input-button" onClick={() => onChange({ yt_dlp_path: "" })}>清除</button>}</div></SettingRow>
    <SettingRow label="自定义 ffmpeg.exe"><div className="input-group"><input value={value.ffmpeg_path} onChange={(event) => onChange({ ffmpeg_path: event.target.value })} placeholder="留空则自动检测" /><button className="input-button" onClick={() => void chooseFfmpeg()}>选择文件</button>{value.ffmpeg_path && <button className="input-button" onClick={() => onChange({ ffmpeg_path: "", ffprobe_path: "" })}>清除</button>}</div></SettingRow>
    <SettingRow label="YouTube PO Token"><div className="input-group"><input value={value.youtube_po_token || ""} onChange={(event) => onChange({ youtube_po_token: event.target.value })} placeholder="格式如 mweb.gvs+... 留空使用默认回退" />{value.youtube_po_token && <button className="input-button" onClick={() => onChange({ youtube_po_token: "" })}>清除</button>}</div></SettingRow>
  </div>;
}
function MediaToolsCard({ status, onStatus, compact = false, required }: { status: ToolStatus; onStatus: (value: ToolStatus) => void; compact?: boolean; required?: ToolComponent }) {
  const components: ToolComponent[] = required ? [required] : ["yt-dlp", "ffmpeg"];
  return <div className={compact ? "media-tools-stack compact" : "media-tools-stack"}>
    {components.map((component) => <MediaToolComponentCard key={component} component={component} status={status} onStatus={onStatus} compact={compact} />)}
  </div>;
}

function MediaToolComponentCard({ component, status, onStatus, compact }: { component: ToolComponent; status: ToolStatus; onStatus: (value: ToolStatus) => void; compact: boolean }) {
  const [successMsg, setSuccessMsg] = useState("");
  const isYtDlp = component === "yt-dlp";
  const available = isYtDlp ? status.yt_dlp_available : status.ffmpeg_available;
  const operationForThis = status.active_component === component;
  const active = operationForThis && ["downloading", "verifying", "extracting"].includes(status.state);
  const someInstallActive = Boolean(status.active_component) && ["downloading", "verifying", "extracting"].includes(status.state);
  const phase = operationForThis ? status.state : available ? "ready" : "missing";
  const downloadBytes = isYtDlp ? status.yt_dlp_download_bytes : status.ffmpeg_download_bytes;
  const installEstimate = isYtDlp ? status.yt_dlp_download_bytes : 199 * 1024 * 1024;
  const installedBytes = isYtDlp ? status.yt_dlp_installed_bytes : status.ffmpeg_installed_bytes;
  const version = isYtDlp ? status.yt_dlp_version : status.ffmpeg_version;
  const source = isYtDlp ? status.yt_dlp_source : status.ffmpeg_source;
  const sourceLabel = source === "custom" ? (isYtDlp ? "自定义路径(更新覆盖)" : "自定义路径(只读保护)") : source === "system" ? "系统 PATH(只读保护)" : source === "bundled" ? "应用安装" : "未安装";
  const title = isYtDlp ? "yt-dlp 基础媒体组件" : "FFmpeg 高清合并组件";
  const description = isYtDlp ? "媒体分析、单文件视频和音频下载" : "最高画质音视频合并、转码与格式处理";
  const progress = status.total_bytes ? Math.min(100, status.downloaded_bytes / status.total_bytes * 100) : 0;
  const latestTargetVersion = isYtDlp ? "2026.07.04" : "8.1.2";
  const isLatest = available && version.includes(latestTargetVersion);

  const prevActiveRef = useRef(false);
  useEffect(() => {
    if (prevActiveRef.current && !active && !status.error && available) {
      if (isYtDlp && source === "custom") {
        setSuccessMsg("✓ 已成功下载并覆盖更新至自定义路径下的 yt-dlp.exe");
      } else {
        setSuccessMsg("✓ 组件已成功下载安装并投入使用");
      }
    }
    prevActiveRef.current = active;
  }, [active, status.error, available, isYtDlp, source]);

  const install = async () => {
    setSuccessMsg("");
    try {
      await api.installMediaTool(component);
      onStatus(await api.toolStatus());
    } catch (error) {
      onStatus({ ...status, active_component: component, state: "failed", error: String(error) });
    }
  };

  const remove = async () => { try { await api.removeMediaTool(component); onStatus(await api.toolStatus()); } catch (error) { onStatus({ ...status, active_component: component, state: "failed", error: String(error) }); } };

  return <div className={compact ? "media-tools-card compact" : "media-tools-card"}>
    <div className="media-tools-card-main">
      <div className="tool-summary">
        <span className={`tool-state ${phase}`}>{available && !active ? <Check size={14} /> : active ? <LoaderCircle className="spin" size={14} /> : <Video size={14} />}</span>
        <div><strong>{title}</strong><small>{description} · {version}{available ? source === "bundled" ? ` · 应用占用 ${formatBytes(installedBytes)}` : ` · 使用${sourceLabel}` : ` · 下载约 ${formatBytes(downloadBytes)} · 安装约 ${formatBytes(installEstimate)}`}</small></div>
      </div>
      <div className="tool-actions">
        {active ? <button onClick={() => void api.cancelMediaTools()}>取消安装</button> : available && source === "bundled" ? <>
          <button className="danger" disabled={someInstallActive} onClick={() => void remove()}>卸载</button>
          <button className="primary" disabled={someInstallActive} onClick={() => void install()}>{isLatest ? "已是最新版 (重新下载)" : "更新组件"}</button>
        </> : available && isYtDlp && source === "custom" ? (
          isLatest ? <button className="input-button" disabled={someInstallActive} title="当前组件已是最新版本，点击可强制重新下载并覆盖自定义文件" onClick={() => void install()}>已是最新版本 (点击重新覆盖)</button>
          : <button className="primary" disabled={someInstallActive} title="发现新版本，点击将下载并覆盖您的自定义路径 yt-dlp.exe" onClick={() => void install()}>覆盖更新自定义</button>
        ) : available ? <button disabled title="已使用第三方外部组件，软件将保持原样，不修改外部文件">使用外部组件</button> : <button className="primary" disabled={someInstallActive} onClick={() => void install()}>下载并安装</button>}
      </div>
    </div>
    {active && <div className="tool-progress"><div><i style={{ width: `${progress}%` }} /></div><span>{status.state === "verifying" ? "正在校验 SHA-256" : status.state === "extracting" ? "正在安全解压" : `${formatBytes(status.downloaded_bytes)} / ${formatBytes(status.total_bytes)}`}</span></div>}
    {successMsg && <p className="tool-success" style={{ color: "#10b981", fontSize: "12px", marginTop: "6px", fontWeight: 500 }}>{successMsg}</p>}
    {operationForThis && status.error && <p className="tool-error">{status.error}</p>}
  </div>;
}

function MediaToolsUpdateRow({ tools, onStatus }: { tools: ToolStatus | null | undefined; onStatus: (status: ToolStatus) => void }) {
  const [checking, setChecking] = useState(false);
  const [updateMsg, setUpdateMsg] = useState("");

  const handleManualCheck = async () => {
    setChecking(true);
    setUpdateMsg("");
    try {
      const currentStatus = await api.toolStatus();
      onStatus(currentStatus);
      const isYtCustom = currentStatus.yt_dlp_source === "custom";
      const isFfExternal = currentStatus.ffmpeg_source === "custom" || currentStatus.ffmpeg_source === "system";

      const ytLocal = currentStatus.yt_dlp_version || "未安装";
      const ytLatest = "2026.07.04";
      const ytIsLatest = currentStatus.yt_dlp_available && ytLocal.includes(ytLatest);
      const ytMsg = `yt-dlp (本地版本: ${ytLocal} / 最新版本: ${ytLatest} · ${ytIsLatest ? (isYtCustom ? "已是最新版，使用自定义路径" : "已是最新版") : currentStatus.yt_dlp_available ? "可更新" : "未安装"})`;

      const ffLocal = currentStatus.ffmpeg_version || "未安装";
      const ffLatest = "8.1.2 essentials";
      const ffIsLatest = currentStatus.ffmpeg_available && ffLocal.includes("8.1.2");
      const ffSourceTag = currentStatus.ffmpeg_source === "system" ? "系统 PATH 自动检测到，已自动复用" : currentStatus.ffmpeg_source === "custom" ? "自定义路径只读保护" : ffIsLatest ? "已是最新版" : "未安装";
      const ffMsg = `FFmpeg (本地检测: ${ffLocal} / 软件推荐: ${ffLatest} · ${ffSourceTag})`;

      setUpdateMsg(`检测完成：${ytMsg}；${ffMsg}`);
    } catch (error) {
      setUpdateMsg(`检测失败：${String(error)}`);
    } finally {
      setChecking(false);
    }
  };

  return <div className="settings-group-content media-path-settings" style={{ marginTop: "12px" }}>
    <div className="media-detect-row">
      <div>
        <strong>检查媒体组件更新</strong>
        <span className="media-detect-desc">手动检测本地与最新版本对比，并识别更新覆盖规则</span>
      </div>
      <button className="input-button primary" disabled={checking || Boolean(tools?.active_component)} onClick={() => void handleManualCheck()}>
        {checking ? "检查中…" : "手动检查是否最新版"}
      </button>
    </div>
    {updateMsg && <p className="media-detect-result" role="status" style={{ width: "100%", marginTop: "8px" }}>{updateMsg}</p>}
  </div>;
}

// ===== 分类规则面板（Task 11）=====
const CATEGORY_RULE_TYPE_LABELS: Record<CategoryRuleType, string> = {
  domain: "域名",
  mime: "MIME 主类型",
  regex: "文件名正则",
};

function newRuleId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `rule-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function emptyRule(priority: number): CategoryRule {
  return {
    id: newRuleId(),
    name: "",
    rule_type: "domain",
    pattern: "",
    target_directory: "",
    enabled: true,
    priority,
  };
}

function CategoryRulesPanel({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [rules, setRules] = useState<CategoryRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<CategoryRule | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testUrl, setTestUrl] = useState("");
  const [testFileName, setTestFileName] = useState("");
  const [testContentType, setTestContentType] = useState("");
  const [testResult, setTestResult] = useState<{ matched: boolean; target_directory: string } | null>(null);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.categoryRuleList();
      setRules(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void reload(); }, []);

  const startAdd = () => {
    const maxPriority = rules.reduce((max, r) => Math.max(max, r.priority), -1);
    setEditing(emptyRule(maxPriority + 1));
    setIsNew(true);
  };
  const startEdit = (rule: CategoryRule) => {
    setEditing({ ...rule });
    setIsNew(false);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) { notify("规则名称不能为空", "error"); return; }
    if (!editing.pattern.trim()) { notify("匹配模式不能为空", "error"); return; }
    if (!editing.target_directory.trim()) { notify("目标目录不能为空", "error"); return; }
    try {
      if (isNew) {
        await api.categoryRuleAdd(editing);
        notify("已新增分类规则");
      } else {
        await api.categoryRuleUpdate(editing);
        notify("已更新分类规则");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeRule = async (id: string) => {
    if (!confirm("确定删除此分类规则？")) return;
    try {
      await api.categoryRuleDelete(id);
      notify("已删除分类规则");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (rule: CategoryRule) => {
    const updated = { ...rule, enabled: !rule.enabled };
    try {
      await api.categoryRuleUpdate(updated);
      setRules((list) => list.map((r) => (r.id === rule.id ? updated : r)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const movePriority = async (rule: CategoryRule, direction: -1 | 1) => {
    // 与相邻规则交换 priority（按当前列表顺序，已按 priority 升序）
    const sorted = [...rules].sort((a, b) => a.priority - b.priority);
    const index = sorted.findIndex((r) => r.id === rule.id);
    if (index < 0) return;
    const targetIndex = direction === -1 ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= sorted.length) return;
    const other = sorted[targetIndex];
    const updatedSelf = { ...rule, priority: other.priority };
    const updatedOther = { ...other, priority: rule.priority };
    try {
      await api.categoryRuleUpdate(updatedSelf);
      await api.categoryRuleUpdate(updatedOther);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const runTest = async () => {
    if (!editing) return;
    if (!testUrl.trim() && !testFileName.trim()) {
      notify("请输入 URL 或文件名以测试规则", "error");
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const result = await api.categoryRuleTest(editing, testUrl.trim(), testFileName.trim(), testContentType.trim() || undefined);
      setTestResult(result);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };

  return <SettingsGroup title="分类规则">
    <p className="settings-note">按优先级匹配域名、文件类型或文件名正则，自动设置新任务的保存目录（不覆盖手动指定的目录）。</p>
    <div className="category-rules-toolbar">
      <button className="input-button" onClick={startAdd}><Plus size={13} /><span>新增规则</span></button>
      <button className="input-button" onClick={() => void reload()}><RefreshCw size={13} /><span>刷新</span></button>
    </div>
    {loading ? <LoaderCircle className="spin" /> : rules.length === 0 ? <p className="settings-note">暂无分类规则。</p> : (
      <div className="category-rules-list" role="table">
        <div className="category-rule-row category-rule-row-header" role="row">
          <span className="category-rule-priority">优先级</span>
          <span className="category-rule-name">名称</span>
          <span className="category-rule-type">类型</span>
          <span className="category-rule-pattern">模式</span>
          <span className="category-rule-target">目标目录</span>
          <span className="category-rule-enabled">启用</span>
          <span className="category-rule-actions">操作</span>
        </div>
        {rules.map((rule, index) => (
          <div key={rule.id} className="category-rule-row" role="row">
            <span className="category-rule-priority" role="cell">{rule.priority}</span>
            <span className="category-rule-name" role="cell" title={rule.name}>{rule.name}</span>
            <span className="category-rule-type" role="cell">{CATEGORY_RULE_TYPE_LABELS[rule.rule_type]}</span>
            <span className="category-rule-pattern" role="cell" title={rule.pattern}><code>{rule.pattern}</code></span>
            <span className="category-rule-target" role="cell" title={rule.target_directory}>{rule.target_directory}</span>
            <span className="category-rule-enabled" role="cell">
              <input type="checkbox" className="toggle" checked={rule.enabled} onChange={() => void toggleEnabled(rule)} aria-label={`${rule.name} 启用状态`} />
            </span>
            <span className="category-rule-actions" role="cell">
              <button title="上移" disabled={index === 0} onClick={() => void movePriority(rule, -1)}><ChevronUp size={13} /></button>
              <button title="下移" disabled={index === rules.length - 1} onClick={() => void movePriority(rule, 1)}><ChevronDown size={13} /></button>
              <button title="编辑" onClick={() => startEdit(rule)}>编辑</button>
              <button title="删除" className="danger" onClick={() => void removeRule(rule.id)}><Trash2 size={13} /></button>
            </span>
          </div>
        ))}
      </div>
    )}
    {editing && (
      <Modal
        title={isNew ? "新增分类规则" : "编辑分类规则"}
        headerAction={
          <label className="dialog-header-action">
            <span>启用此规则</span>
            <input className="toggle" type="checkbox" checked={editing.enabled} onChange={(e) => setEditing({ ...editing, enabled: e.target.checked })} />
          </label>
        }
        onClose={() => setEditing(null)}
        style={{ width: "520px" }}
      >
        <div className="category-rule-edit-form">

          <div className="form-grid-2">
            <Field label="名称">
              <input value={editing.name} onChange={(e) => setEditing({ ...editing, name: e.target.value })} placeholder="例如：GitHub 仓库" />
            </Field>
            <Field label="匹配类型">
              <Select
                value={editing.rule_type}
                onChange={(val: any) => setEditing({ ...editing, rule_type: val as CategoryRuleType })}
                options={[
                  { value: "domain", label: "域名（支持子域名）" },
                  { value: "mime", label: "MIME 主类型" },
                  { value: "regex", label: "文件名正则" },
                ]}
                ariaLabel="匹配类型"
              />
            </Field>
          </div>

          <div className="form-grid-2">
            <Field label={editing.rule_type === "domain" ? "域名（如 github.com）" : editing.rule_type === "mime" ? "主类型（如 video）" : "正则表达式（如 \\.mp4$）"}>
              <input value={editing.pattern} onChange={(e) => setEditing({ ...editing, pattern: e.target.value })} placeholder={editing.rule_type === "domain" ? "github.com" : editing.rule_type === "mime" ? "video" : "\\.mp4$"} />
            </Field>
            <Field label="优先级（越小越先）">
              <input type="number" value={editing.priority} onChange={(e) => setEditing({ ...editing, priority: +e.target.value })} />
            </Field>
          </div>

          <Field label="目标目录">
            <div className="input-group">
              <input value={editing.target_directory} onChange={(e) => setEditing({ ...editing, target_directory: e.target.value })} placeholder="例如：D:\\Downloads\\GitHub" />
              <button className="input-button" onClick={async () => {
                const picked = await pickPath({ directory: true, multiple: false, title: "选择目标目录" });
                if (typeof picked === "string") setEditing({ ...editing, target_directory: picked });
              }}>选择目录</button>
            </div>
          </Field>

          <details className="category-rule-test-details">
            <summary style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <span style={{ display: "inline-flex", alignItems: "center", gap: "6px" }}><Sparkles size={12} /> 测试规则匹配</span>
              {testResult && (
                <small className={`category-rule-test-badge ${testResult.matched ? "ok" : "miss"}`} style={{ fontSize: "10px", padding: "1px 6px", borderRadius: "4px", fontWeight: "normal", background: testResult.matched ? "rgba(52, 199, 89, 0.15)" : "rgba(255, 59, 48, 0.15)", color: testResult.matched ? "var(--success)" : "var(--danger)" }}>
                  {testResult.matched ? "✓ 已命中" : "× 未命中"}
                </small>
              )}
            </summary>
            <div className="category-rule-test-body">
              <Field label="测试 URL"><input value={testUrl} onChange={(e) => setTestUrl(e.target.value)} placeholder="https://api.github.com/users/octocat" /></Field>
              <div className="form-grid-2">
                <Field label="测试文件名"><input value={testFileName} onChange={(e) => setTestFileName(e.target.value)} placeholder="octocat.json" /></Field>
                <Field label="Content-Type（可选）"><input value={testContentType} onChange={(e) => setTestContentType(e.target.value)} placeholder="application/json" /></Field>
              </div>
              <div className="category-rule-test-actions">
                <button className="input-button primary-border" disabled={testing} onClick={() => void runTest()}>{testing ? "测试中…" : "测试命中"}</button>
                <button className="input-button" type="button" onClick={() => {
                  const rawPat = editing.pattern.trim();
                  let sampleUrl = "";
                  let sampleFile = "";
                  let sampleMime = "";

                  if (editing.rule_type === "domain") {
                    let dom = rawPat.replace(/^https?:\/\//i, "").split("/")[0].trim();
                    dom = dom || "github.com";
                    sampleUrl = `https://${dom}/archive/download_sample.zip`;
                    sampleFile = "download_sample.zip";
                    sampleMime = "application/zip";
                  } else if (editing.rule_type === "mime") {
                    let mime = rawPat.split("/")[0].trim().toLowerCase() || "video";
                    const ext = mime === "video" ? "mp4" : mime === "image" ? "png" : mime === "audio" ? "mp3" : "bin";
                    sampleUrl = `https://example.com/media/sample_file.${ext}`;
                    sampleFile = `sample_file.${ext}`;
                    sampleMime = rawPat.includes("/") ? rawPat : `${mime}/octet-stream`;
                  } else {
                    let sampleName = "download_sample.zip";
                    if (rawPat) {
                      const extMatch = /\.(mp4|mkv|avi|mov|mp3|flac|wav|zip|rar|7z|tar|gz|exe|msi|pdf|epub|png|jpg|jpeg|webp)\b/i.exec(rawPat);
                      if (extMatch) {
                        sampleName = `sample_file${extMatch[0]}`;
                      } else {
                        const cleaned = rawPat.replace(/[\^$\\().*+?\[\]{}|]/g, "").trim();
                        if (cleaned) sampleName = cleaned;
                      }
                    }
                    sampleUrl = `https://example.com/files/${sampleName}`;
                    sampleFile = sampleName;
                    sampleMime = "application/octet-stream";
                  }

                  setTestUrl(sampleUrl);
                  setTestFileName(sampleFile);
                  setTestContentType(sampleMime);

                  // 自动顺便触发一次测试，直接显示命中状态
                  void (async () => {
                    setTesting(true);
                    try {
                      const res = await api.categoryRuleTest(editing, sampleUrl, sampleFile, sampleMime || undefined);
                      setTestResult(res);
                    } catch {
                      setTestResult({ matched: false, target_directory: "" });
                    } finally {
                      setTesting(false);
                    }
                  })();
                }}>填入示例并测试</button>
                {testResult && (
                  <span className={`category-rule-test-result ${testResult.matched ? "ok" : "miss"}`} role="status">
                    {testResult.matched ? <>命中 · 目标目录：<code>{testResult.target_directory}</code></> : "未命中"}
                  </span>
                )}
              </div>
            </div>
          </details>

          <div className="dialog-actions">
            <button onClick={() => setEditing(null)}>取消</button>
            <button className="primary" onClick={() => void saveEdit()}>保存</button>
          </div>
        </div>
      </Modal>
    )}
  </SettingsGroup>;
}

// Task 20: 文件名清理规则管理面板。复用 CategoryRulesPanel 的列表/Modal 模式。
function FilenameCleanupPanel({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [rules, setRules] = useState<FilenameCleanupRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<FilenameCleanupRule | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testFileName, setTestFileName] = useState("");
  const [testResult, setTestResult] = useState<string | null>(null);

  // 内置规则 ID 不可删除（与后端 seed_builtin_filename_cleanup_rules 对应）
  const BUILTIN_RULE_IDS = new Set([
    "remove-bracket-site",
    "remove-chinese-bracket-site",
    "remove-chinese-bracket-promo",
    "remove-paren-quality",
    "remove-square-bracket-quality",
    "remove-media-codec-tags",
    "remove-underscore-site",
    "remove-copy-suffix",
    "collapse-spaces",
    "strip-trailing-spaces",
  ]);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.filenameCleanupRuleList();
      setRules(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void reload(); }, []);

  const startAdd = () => {
    const maxPriority = rules.reduce((max, r) => Math.max(max, r.priority), 39);
    const empty: FilenameCleanupRule = {
      id: `custom-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: "",
      pattern: "",
      replacement: "",
      enabled: true,
      priority: maxPriority + 10,
    };
    setEditing(empty);
    setIsNew(true);
    setTestResult(null);
  };
  const startEdit = (rule: FilenameCleanupRule) => {
    setEditing({ ...rule });
    setIsNew(false);
    setTestResult(null);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) { notify("规则名称不能为空", "error"); return; }
    if (!editing.pattern.trim()) { notify("正则模式不能为空", "error"); return; }
    if (!editing.id.trim()) { notify("规则 ID 不能为空", "error"); return; }
    try {
      if (isNew) {
        await api.filenameCleanupRuleAdd(editing);
        notify("已新增文件名清理规则");
      } else {
        await api.filenameCleanupRuleUpdate(editing);
        notify("已更新文件名清理规则");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeRule = async (id: string) => {
    if (BUILTIN_RULE_IDS.has(id)) {
      notify("内置规则不可删除，可禁用或编辑", "error");
      return;
    }
    if (!confirm("确定删除此文件名清理规则？")) return;
    try {
      await api.filenameCleanupRuleDelete(id);
      notify("已删除文件名清理规则");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (rule: FilenameCleanupRule) => {
    const updated = { ...rule, enabled: !rule.enabled };
    try {
      await api.filenameCleanupRuleUpdate(updated);
      setRules((list) => list.map((r) => (r.id === rule.id ? updated : r)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const movePriority = async (rule: FilenameCleanupRule, direction: -1 | 1) => {
    const sorted = [...rules].sort((a, b) => a.priority - b.priority);
    const index = sorted.findIndex((r) => r.id === rule.id);
    if (index < 0) return;
    const targetIndex = direction === -1 ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= sorted.length) return;
    const other = sorted[targetIndex];
    const updatedSelf = { ...rule, priority: other.priority };
    const updatedOther = { ...other, priority: rule.priority };
    try {
      await api.filenameCleanupRuleUpdate(updatedSelf);
      await api.filenameCleanupRuleUpdate(updatedOther);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const runTest = async () => {
    if (!editing) return;
    if (!testFileName.trim()) {
      notify("请输入测试文件名", "error");
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      // 测试时仅应用当前编辑中的规则（保持启用状态原样，便于预览禁用规则的效果）
      const result = await api.filenameCleanupPreview(testFileName.trim(), [editing]);
      setTestResult(result);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };

  return <SettingsGroup title="文件名清理规则">
    <p className="settings-note">
      保存文件前按优先级进行正则替换，用以去除水印、标记或多余空格。仅对未手动编辑文件名的任务生效。
    </p>
    <div className="category-rules-toolbar">
      <button className="input-button" onClick={startAdd}><Plus size={13} /><span>新增规则</span></button>
      <button className="input-button" onClick={() => void reload()}><RefreshCw size={13} /><span>刷新</span></button>
    </div>
    {loading ? <LoaderCircle className="spin" /> : rules.length === 0 ? <p className="settings-note">暂无文件名清理规则。</p> : (
      <div className="category-rules-list" role="table">
        <div className="category-rule-row category-rule-row-header filename-cleanup-row" role="row">
          <span className="category-rule-priority">优先级</span>
          <span className="category-rule-name">名称</span>
          <span className="category-rule-pattern">正则模式</span>
          <span className="category-rule-target">替换为</span>
          <span className="category-rule-enabled">启用</span>
          <span className="category-rule-actions">操作</span>
        </div>
        {rules.map((rule, index) => (
          <div key={rule.id} className="category-rule-row filename-cleanup-row" role="row">
            <span className="category-rule-priority" role="cell">{rule.priority}</span>
            <span className="category-rule-name" role="cell" title={rule.name}>
              {rule.name}
              {BUILTIN_RULE_IDS.has(rule.id) && <small className="preset-builtin-badge">内置</small>}
            </span>
            <span className="category-rule-pattern" role="cell" title={rule.pattern}><code>{rule.pattern}</code></span>
            <span className="category-rule-target" role="cell" title={rule.replacement || "（删除匹配内容）"}>{rule.replacement || "（删除）"}</span>
            <span className="category-rule-enabled" role="cell">
              <input type="checkbox" className="toggle" checked={rule.enabled} onChange={() => void toggleEnabled(rule)} aria-label={`${rule.name} 启用状态`} />
            </span>
            <span className="category-rule-actions" role="cell">
              <button title="上移" disabled={index === 0} onClick={() => void movePriority(rule, -1)}><ChevronUp size={13} /></button>
              <button title="下移" disabled={index === rules.length - 1} onClick={() => void movePriority(rule, 1)}><ChevronDown size={13} /></button>
              <button title="编辑" onClick={() => startEdit(rule)}>编辑</button>
              <button title={BUILTIN_RULE_IDS.has(rule.id) ? "内置规则不可删除" : "删除"} className="danger" disabled={BUILTIN_RULE_IDS.has(rule.id)} onClick={() => void removeRule(rule.id)}><Trash2 size={13} /></button>
            </span>
          </div>
        ))}
      </div>
    )}
    {editing && (
      <Modal
        title={isNew ? "新增文件名清理规则" : "编辑文件名清理规则"}
        headerAction={
          <label className="dialog-header-action">
            <span>启用规则</span>
            <input className="toggle" type="checkbox" checked={editing.enabled} onChange={(e) => setEditing({ ...editing, enabled: e.target.checked })} />
          </label>
        }
        onClose={() => setEditing(null)}
        style={{ width: "520px" }}
      >
        <div className="category-rule-edit-form">
          <div className="form-row-group">
            <Field label="名称">
              <input value={editing.name} onChange={(e) => setEditing({ ...editing, name: e.target.value })} placeholder="例如：去除站点方括号" />
            </Field>
            <Field label="优先级（越小越先）">
              <input type="number" value={editing.priority} onChange={(e) => setEditing({ ...editing, priority: +e.target.value })} />
            </Field>
          </div>
          <div className="form-row-group">
            <Field label="正则模式（regex crate 语法）">
              <input value={editing.pattern} onChange={(e) => setEditing({ ...editing, pattern: e.target.value })} placeholder="\\[(www\\.)?[\\w.-]+\\]" />
            </Field>
            <Field label="替换为（留空为删除）">
              <input value={editing.replacement} onChange={(e) => setEditing({ ...editing, replacement: e.target.value })} placeholder="例如：$1 或空" />
            </Field>
          </div>
          <div className="category-rule-test-section">
            <h3>测试规则</h3>
            <div className="test-inline-row">
              <input value={testFileName} onChange={(e) => setTestFileName(e.target.value)} placeholder="movie [www.example.com] (1080p).mp4" />
              <button className="input-button" disabled={testing} onClick={() => void runTest()}>{testing ? "测试中…" : "测试清理"}</button>
            </div>
            {testResult !== null && (
              <p className="category-rule-test-result ok" role="status">
                清理结果：<code>{testResult || "（空）"}</code>
              </p>
            )}
          </div>
          <div className="dialog-actions"><button onClick={() => setEditing(null)}>取消</button><button className="primary" onClick={() => void saveEdit()}>保存</button></div>
        </div>
      </Modal>
    )}
  </SettingsGroup>;
}

// Task 43: 平台命名模板管理面板。复用 FilenameCleanupPanel 的列表/Modal 模式。
// 内置模板可编辑、可禁用，但不可删除（前端隐藏删除按钮，与后端 seed 对应）。
function PlatformNamingTemplatePanel({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [templates, setTemplates] = useState<PlatformNamingTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<PlatformNamingTemplate | null>(null);
  const [isNew, setIsNew] = useState(false);

  // 平台 key → 中文名映射（与后端 MediaPlatform::display_name 对应）。
  // 用于列表展示与编辑表单的下拉选项。
  const PLATFORM_LABELS: Array<[string, string]> = [
    ["douyin", "抖音"],
    ["tiktok", "TikTok"],
    ["twitter", "Twitter/X"],
    ["youtube", "YouTube"],
    ["bilibili", "哔哩哔哩"],
    ["weibo", "微博"],
    ["unknown", "未知平台"],
  ];
  const platformLabel = (key: string): string =>
    PLATFORM_LABELS.find(([k]) => k === key)?.[1] ?? key;

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.platformNamingTemplateList();
      setTemplates(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void reload(); }, []);

  const startAdd = () => {
    const empty: PlatformNamingTemplate = {
      id: `template-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      platform: "douyin",
      template: "{author}_{title}_{date}",
      enabled: true,
      is_builtin: false,
    };
    setEditing(empty);
    setIsNew(true);
  };
  const startEdit = (template: PlatformNamingTemplate) => {
    setEditing({ ...template });
    setIsNew(false);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.id.trim()) { notify("模板 ID 不能为空", "error"); return; }
    if (!editing.platform.trim()) { notify("平台不能为空", "error"); return; }
    if (!editing.template.trim()) { notify("模板内容不能为空", "error"); return; }
    try {
      if (isNew) {
        await api.platformNamingTemplateAdd(editing);
        notify("已新增平台命名模板");
      } else {
        await api.platformNamingTemplateUpdate(editing);
        notify("已更新平台命名模板");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeTemplate = async (template: PlatformNamingTemplate) => {
    if (template.is_builtin) {
      notify("内置模板不可删除，可禁用或编辑", "error");
      return;
    }
    if (!confirm("确定删除此平台命名模板？")) return;
    try {
      await api.platformNamingTemplateDelete(template.id);
      notify("已删除平台命名模板");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (template: PlatformNamingTemplate) => {
    const updated = { ...template, enabled: !template.enabled };
    try {
      await api.platformNamingTemplateUpdate(updated);
      setTemplates((list) => list.map((t) => (t.id === template.id ? updated : t)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  // 实时预览：基于编辑中的模板字符串与示例变量计算预览文件名（不含扩展名）。
  // 不调用后端 API，纯前端字符串替换，便于即时反馈。
  const previewFileName = useMemo(() => {
    if (!editing) return "";
    const sampleVars: Record<string, string> = {
      author: "张三",
      title: "示例标题",
      date: "20260720",
      platform: editing.platform,
      id: "7012345678901234567",
      channel: "示例频道",
      bvid: "BV1xx411c7mD",
    };
    let result = editing.template.replace(/\{(author|title|date|platform|id|channel|bvid)\}/g, (_, key: string) => sampleVars[key] ?? "");
    // 压缩连续下划线，去除首尾下划线（与后端 sanitize_filename 语义一致）
    result = result.replace(/_+/g, "_").replace(/^_+|_+$/g, "");
    return result || "media";
  }, [editing]);

  return <SettingsGroup title="平台命名模板">
    <p className="settings-note">
      媒体下载完成后套用此文件名模板。仅对新建任务生效，支持变量替换，限制在 100 字符内。
    </p>
    <div className="category-rules-toolbar">
      <button className="input-button" onClick={startAdd}><Plus size={13} /><span>新增模板</span></button>
      <button className="input-button" onClick={() => void reload()}><RefreshCw size={13} /><span>刷新</span></button>
    </div>
    {loading ? <LoaderCircle className="spin" /> : templates.length === 0 ? <p className="settings-note">暂无平台命名模板。</p> : (
      <div className="category-rules-list" role="table">
        <div className="category-rule-row category-rule-row-header" role="row">
          <span className="category-rule-name">平台</span>
          <span className="category-rule-pattern">模板</span>
          <span className="category-rule-enabled">启用</span>
          <span className="category-rule-actions">操作</span>
        </div>
        {templates.map((template) => (
          <div key={template.id} className="category-rule-row" role="row">
            <span className="category-rule-name" role="cell" title={template.platform}>
              {platformLabel(template.platform)}
              {template.is_builtin && <small className="preset-builtin-badge">内置</small>}
            </span>
            <span className="category-rule-pattern" role="cell" title={template.template}><code>{template.template}</code></span>
            <span className="category-rule-enabled" role="cell">
              <input type="checkbox" className="toggle" checked={template.enabled} onChange={() => void toggleEnabled(template)} aria-label={`${platformLabel(template.platform)} 模板启用状态`} />
            </span>
            <span className="category-rule-actions" role="cell">
              <button title="编辑" onClick={() => startEdit(template)}>编辑</button>
              <button title={template.is_builtin ? "内置模板不可删除" : "删除"} className="danger" disabled={template.is_builtin} onClick={() => void removeTemplate(template)}><Trash2 size={13} /></button>
            </span>
          </div>
        ))}
      </div>
    )}
    {editing && (
      <Modal title={isNew ? "新增平台命名模板" : "编辑平台命名模板"} onClose={() => setEditing(null)} style={{ width: "540px" }}>
        <div className="category-rule-edit-form">
          <Field label="平台">
            <Select
              value={editing.platform}
              onChange={(val: any) => setEditing({ ...editing, platform: String(val) })}
              options={PLATFORM_LABELS.map(([key, label]) => ({
                value: key,
                label,
              }))}
              ariaLabel="平台"
            />
          </Field>
          <Field label="模板字符串">
            <input value={editing.template} onChange={(e) => setEditing({ ...editing, template: e.target.value })} placeholder="{author}_{title}_{date}" />
          </Field>
          <label className="setting-row">
            <div><strong>启用</strong></div>
            <input className="toggle" type="checkbox" checked={editing.enabled} onChange={(e) => setEditing({ ...editing, enabled: e.target.checked })} />
          </label>
          <div className="category-rule-test-section">
            <h3>变量说明</h3>
            <ul className="settings-note" style={{ paddingLeft: "20px", lineHeight: 1.7 }}>
              <li><code>{"{author}"}</code>：作者/上传者昵称（yt-dlp uploader/channel/uploader_id 优先级回退）</li>
              <li><code>{"{title}"}</code>：媒体标题</li>
              <li><code>{"{date}"}</code>：上传日期（YYYYMMDD 格式）</li>
              <li><code>{"{platform}"}</code>：平台 key（如 douyin / youtube）</li>
              <li><code>{"{id}"}</code>：站点视频 ID（如推文 ID、YouTube 视频 ID）</li>
              <li><code>{"{channel}"}</code>：频道名（YouTube 等平台有意义，与 author 区分）</li>
              <li><code>{"{bvid}"}</code>：B 站 BV 号（B 站场景下为 display_id）</li>
            </ul>
            <p className="settings-note">
              未知变量（如 <code>{"{foo}"}</code>）原样保留；缺失的已知变量替换为空。
              非法字符 <code>\ / : * ? &quot; &lt; &gt; |</code> 与控制字符替换为 <code>_</code>，压缩连续下划线。
            </p>
            <h3>预览（示例变量）</h3>
            <p className="category-rule-test-result ok" role="status">
              预览文件名：<code>{previewFileName}</code>
            </p>
          </div>
          <div className="dialog-actions"><button onClick={() => setEditing(null)}>取消</button><button className="primary" onClick={() => void saveEdit()}>保存</button></div>
        </div>
      </Modal>
    )}
  </SettingsGroup>;
}

// Task 12: 下载预设管理面板。复用 CategoryRulesPanel 的列表/Modal 模式。
function PresetsPanel({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [presets, setPresets] = useState<DownloadPreset[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<DownloadPreset | null>(null);
  const [isNew, setIsNew] = useState(false);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.presetList();
      setPresets(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void reload(); }, []);

  const actionLabel = (action?: CompletionAction | null): string => {
    if (!action || action === "none") return "无";
    if (action === "open-folder") return "打开文件夹";
    if (action === "run-file") return "运行文件";
    if (action === "shutdown") return "关机";
    if (action === "hibernate") return "休眠";
    return "无";
  };

  const startAdd = () => {
    const newPreset: DownloadPreset = {
      id: `custom-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: "",
      connections: 8,
      speed_limit: null,
      completion_action: null,
      verify_checksum: false,
      scheduled_at: null,
      is_builtin: false,
    };
    setEditing(newPreset);
    setIsNew(true);
  };

  const startEdit = (preset: DownloadPreset) => {
    setEditing({ ...preset });
    setIsNew(false);
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) { notify("预设名称不能为空", "error"); return; }
    if (!editing.id.trim()) { notify("预设 ID 不能为空", "error"); return; }
    if (![1, 2, 4, 8, 16, 32].includes(editing.connections)) {
      notify("连接数只能是 1 / 2 / 4 / 8 / 16 / 32", "error");
      return;
    }
    if (editing.scheduled_at && !/^\d{1,2}:\d{2}$/.test(editing.scheduled_at.trim())) {
      notify("计划时间格式应为 HH:MM（24 小时制）", "error");
      return;
    }
    try {
      if (isNew) {
        await api.presetAdd(editing);
        notify("已新增下载预设");
      } else {
        await api.presetUpdate(editing);
        notify("已更新下载预设");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removePreset = async (id: string) => {
    if (!confirm("确定删除此下载预设？")) return;
    try {
      await api.presetDelete(id);
      notify("已删除下载预设");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  return <SettingsGroup title="下载预设">
    <p className="settings-note">
      预设用于快速套用一组下载参数。内置预设可编辑但不可删除，自定义预设可任意增删改。
    </p>
    <div className="category-rules-toolbar">
      <button className="input-button" onClick={startAdd}><Plus size={13} /><span>新增预设</span></button>
      <button className="input-button" onClick={() => void reload()}><RefreshCw size={13} /><span>刷新</span></button>
    </div>
    {loading ? <LoaderCircle className="spin" /> : presets.length === 0 ? <p className="settings-note">暂无下载预设。</p> : (
      <div className="category-rules-list" role="table">
        <div className="category-rule-row category-rule-row-header preset-row" role="row">
          <span>名称</span>
          <span>连接数</span>
          <span>限速</span>
          <span>完成动作</span>
          <span>校验</span>
          <span>计划时间</span>
          <span>操作</span>
        </div>
        {presets.map((preset) => (
          <div key={preset.id} className="category-rule-row preset-row" role="row">
            <span role="cell" title={preset.name}>
              {preset.name}
              {preset.is_builtin && <span className="preset-builtin-badge">内置</span>}
            </span>
            <span className="preset-connections" role="cell">{preset.connections} 路</span>
            <span role="cell">{preset.speed_limit ? `${Math.round(preset.speed_limit / 1024)} KB/s` : "不限速"}</span>
            <span role="cell">{actionLabel(preset.completion_action)}</span>
            <span className="preset-verify" role="cell">{preset.verify_checksum ? "是" : "否"}</span>
            <span className="preset-scheduled" role="cell">{preset.scheduled_at || "—"}</span>
            <span className="category-rule-actions" role="cell">
              <button title="编辑" onClick={() => startEdit(preset)}>编辑</button>
              <button title="删除" className="danger" disabled={preset.is_builtin} onClick={() => void removePreset(preset.id)} aria-label={`删除预设 ${preset.name}`}><Trash2 size={13} /></button>
            </span>
          </div>
        ))}
      </div>
    )}
    {editing && (
      <Modal title={isNew ? "新增下载预设" : "编辑下载预设"} onClose={() => setEditing(null)} style={{ width: "520px" }}>
        <div className="category-rule-edit-form">
          <Field label="ID（不可修改）">
            <input value={editing.id} disabled />
          </Field>
          <Field label="名称">
            <input value={editing.name} onChange={(e) => setEditing({ ...editing, name: e.target.value })} placeholder="例如：影视下载" />
          </Field>
          <Field label="连接数（仅允许 1 / 2 / 4 / 8 / 16 / 32）">
            <Select
              value={editing.connections}
              onChange={(val: any) => setEditing({ ...editing, connections: +val })}
              options={[
                { value: 1, label: "1 路（单连接）" },
                { value: 2, label: "2 路" },
                { value: 4, label: "4 路" },
                { value: 8, label: "8 路（默认）" },
                { value: 16, label: "16 路" },
                { value: 32, label: "32 路（大文件）" },
              ]}
              ariaLabel="连接数"
            />
          </Field>
          <Field label="单任务限速（KB/s，0 表示不限速）">
            <input type="number" min="0" value={editing.speed_limit ? Math.round(editing.speed_limit / 1024) : 0} onChange={(e) => setEditing({ ...editing, speed_limit: +e.target.value ? +e.target.value * 1024 : null })} />
          </Field>
          <Field label="计划时间（HH:MM 24 小时制，留空表示立即开始）">
            <input value={editing.scheduled_at ?? ""} onChange={(e) => setEditing({ ...editing, scheduled_at: e.target.value || null })} placeholder="例如：22:00" />
          </Field>
          <Field className="wide" label="完成后动作">
            <CompletionActionEditor
              value={editing.completion_action ?? "none"}
              onChange={(a) => setEditing({ ...editing, completion_action: a === "none" ? null : a })}
              hidePowerOptions
            />
          </Field>
          <label className="setting-row">
            <div><strong>完成后校验 SHA-256</strong></div>
            <input className="toggle" type="checkbox" checked={editing.verify_checksum} onChange={(e) => setEditing({ ...editing, verify_checksum: e.target.checked })} />
          </label>
          <div className="dialog-actions"><button onClick={() => setEditing(null)}>取消</button><button className="primary" onClick={() => void saveEdit()}>保存</button></div>
        </div>
      </Modal>
    )}
  </SettingsGroup>;
}

/**
 * Task 36: 任务模板管理面板。
 *
 * 列出全部任务模板（按 priority 升序），支持新增/编辑/删除/启用切换/优先级上下移。
 * 模板字段：域名匹配模式、连接数、限速、请求头、保存目录、完成动作。
 * 提供测试框，输入 URL 查看是否命中任意模板。
 *
 * 字段语义：所有字段（除 name/domain_pattern/priority/enabled 外）都是可选覆盖；
 * 留空表示不覆盖新任务的对应字段。后端在 manager.add 流程中按域名匹配并套用。
 */
function TaskTemplatesPanel({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [templates, setTemplates] = useState<TaskTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<TaskTemplate | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testUrl, setTestUrl] = useState("");
  const [testResult, setTestResult] = useState<TaskTemplateTestResult | null>(null);
  // 请求头以多行文本展示，每行 "Key: Value"
  const [headersText, setHeadersText] = useState("");

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.taskTemplateList();
      setTemplates(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void reload(); }, []);

  const startAdd = () => {
    const maxPriority = templates.reduce((max, t) => Math.max(max, t.priority), -1);
    const tpl: TaskTemplate = {
      id: `tpl-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      name: "",
      domain_pattern: "",
      connections: null,
      speed_limit: null,
      headers: null,
      destination: null,
      completion_action: null,
      enabled: true,
      priority: maxPriority + 1,
    };
    setEditing(tpl);
    setHeadersText("");
    setIsNew(true);
  };

  const startEdit = (tpl: TaskTemplate) => {
    setEditing({ ...tpl });
    setHeadersText(
      tpl.headers
        ? Object.entries(tpl.headers).map(([k, v]) => `${k}: ${v}`).join("\n")
        : ""
    );
    setIsNew(false);
  };

  // 解析多行 "Key: Value" 文本为 headers map。空文本返回 null。
  // 格式错误抛出中文错误，由调用方捕获后展示给用户。
  const parseHeaders = (text: string): Record<string, string> | null => {
    const trimmed = text.trim();
    if (!trimmed) return null;
    const result: Record<string, string> = {};
    for (const line of trimmed.split(/\r?\n/)) {
      const lineTrim = line.trim();
      if (!lineTrim) continue;
      const idx = lineTrim.indexOf(":");
      if (idx <= 0) {
        throw new Error(`请求头格式错误：${line}（应为 Key: Value）`);
      }
      const key = lineTrim.slice(0, idx).trim();
      const value = lineTrim.slice(idx + 1).trim();
      if (!key) throw new Error(`请求头键不能为空：${line}`);
      result[key] = value;
    }
    return Object.keys(result).length > 0 ? result : null;
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editing.name.trim()) { notify("模板名称不能为空", "error"); return; }
    if (!editing.domain_pattern.trim()) { notify("域名匹配模式不能为空", "error"); return; }
    if (editing.connections != null && ![1, 2, 4, 8, 16, 32].includes(editing.connections)) {
      notify("连接数只能是 1 / 2 / 4 / 8 / 16 / 32", "error");
      return;
    }
    let headers: Record<string, string> | null;
    try {
      headers = parseHeaders(headersText);
    } catch (error) {
      notify(String(error), "error");
      return;
    }
    const toSave: TaskTemplate = {
      ...editing,
      headers,
      destination: editing.destination?.trim() || null,
    };
    try {
      if (isNew) {
        await api.taskTemplateAdd(toSave);
        notify("已新增任务模板");
      } else {
        await api.taskTemplateUpdate(toSave);
        notify("已更新任务模板");
      }
      setEditing(null);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeTemplate = async (id: string) => {
    if (!confirm("确定删除此任务模板？")) return;
    try {
      await api.taskTemplateDelete(id);
      notify("已删除任务模板");
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const toggleEnabled = async (tpl: TaskTemplate) => {
    const updated = { ...tpl, enabled: !tpl.enabled };
    try {
      await api.taskTemplateUpdate(updated);
      setTemplates((list) => list.map((t) => (t.id === tpl.id ? updated : t)));
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const movePriority = async (tpl: TaskTemplate, direction: -1 | 1) => {
    const sorted = [...templates].sort((a, b) => a.priority - b.priority);
    const index = sorted.findIndex((t) => t.id === tpl.id);
    if (index < 0) return;
    const targetIndex = direction === -1 ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= sorted.length) return;
    const other = sorted[targetIndex];
    const updatedSelf = { ...tpl, priority: other.priority };
    const updatedOther = { ...other, priority: tpl.priority };
    try {
      await api.taskTemplateUpdate(updatedSelf);
      await api.taskTemplateUpdate(updatedOther);
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const runTest = async () => {
    if (!testUrl.trim()) {
      notify("请输入 URL 以测试模板匹配", "error");
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const result = await api.taskTemplateTest(testUrl.trim());
      setTestResult(result);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };

  const fmtSpeed = (v?: number | null): string => (v ? `${Math.round(v / 1024)} KB/s` : "—");
  const fmtConnections = (v?: number | null): string => (v ? `${v} 路` : "—");
  const fmtDestination = (v?: string | null): string => (v && v.trim() ? v : "—");

  const actionLabel = (action?: CompletionAction | null): string => {
    if (!action || action === "none") return "—";
    if (action === "open-folder") return "打开文件夹";
    if (action === "run-file") return "运行文件";
    if (action === "shutdown") return "关机";
    if (action === "hibernate") return "休眠";
    if (action === "quit") return "退出应用";
    if (typeof action === "object" && "run-command" in action) return "运行命令";
    if (typeof action === "object" && "copy-to" in action) return "复制到";
    if (typeof action === "object" && "move-to" in action) return "移动到";
    return "—";
  };

  return <SettingsGroup title="任务模板">
    <p className="settings-note">
      新任务根据域名（支持 *.example.com 通配）自动套用连接数、限速、目录等设置。已手动配置的字段不会被覆盖。
    </p>
    <div className="category-rules-toolbar">
      <button className="input-button" onClick={startAdd}><Plus size={13} /><span>新增模板</span></button>
      <button className="input-button" onClick={() => void reload()}><RefreshCw size={13} /><span>刷新</span></button>
    </div>
    {loading ? <LoaderCircle className="spin" /> : templates.length === 0 ? <p className="settings-note">暂无任务模板。</p> : (
      <div className="category-rules-list" role="table">
        <div className="category-rule-row category-rule-row-header" role="row">
          <span className="category-rule-priority">优先级</span>
          <span className="category-rule-name">名称</span>
          <span className="category-rule-pattern">域名模式</span>
          <span className="preset-connections">连接数</span>
          <span>限速</span>
          <span>保存目录</span>
          <span>完成动作</span>
          <span className="category-rule-enabled">启用</span>
          <span className="category-rule-actions">操作</span>
        </div>
        {templates.map((tpl, index) => (
          <div key={tpl.id} className="category-rule-row" role="row">
            <span className="category-rule-priority" role="cell">{tpl.priority}</span>
            <span className="category-rule-name" role="cell" title={tpl.name}>{tpl.name}</span>
            <span className="category-rule-pattern" role="cell" title={tpl.domain_pattern}><code>{tpl.domain_pattern}</code></span>
            <span className="preset-connections" role="cell">{fmtConnections(tpl.connections)}</span>
            <span role="cell">{fmtSpeed(tpl.speed_limit)}</span>
            <span role="cell" title={tpl.destination ?? ""}>{fmtDestination(tpl.destination)}</span>
            <span role="cell">{actionLabel(tpl.completion_action)}</span>
            <span className="category-rule-enabled" role="cell">
              <input type="checkbox" className="toggle" checked={tpl.enabled} onChange={() => void toggleEnabled(tpl)} aria-label={`${tpl.name} 启用状态`} />
            </span>
            <span className="category-rule-actions" role="cell">
              <button title="上移" disabled={index === 0} onClick={() => void movePriority(tpl, -1)}><ChevronUp size={13} /></button>
              <button title="下移" disabled={index === templates.length - 1} onClick={() => void movePriority(tpl, 1)}><ChevronDown size={13} /></button>
              <button title="编辑" onClick={() => startEdit(tpl)}>编辑</button>
              <button title="删除" className="danger" onClick={() => void removeTemplate(tpl.id)}><Trash2 size={13} /></button>
            </span>
          </div>
        ))}
      </div>
    )}
    <div className="category-rule-test-section" style={{ marginTop: 16 }}>
      <h3>测试模板匹配</h3>
      <Field label="URL"><input value={testUrl} onChange={(e) => setTestUrl(e.target.value)} placeholder="https://api.github.com/users/octocat" /></Field>
      <button className="input-button" disabled={testing} onClick={() => void runTest()}>{testing ? "测试中…" : "测试命中"}</button>
      {testResult && (
        <p className={`category-rule-test-result ${testResult.matched ? "ok" : "miss"}`} role="status">
          {testResult.matched
            ? <>命中 · 模板：<code>{testResult.matched_template_name ?? testResult.matched_template_id}</code></>
            : "未命中"}
        </p>
      )}
    </div>
    {editing && (
      <Modal title={isNew ? "新增任务模板" : "编辑任务模板"} onClose={() => setEditing(null)} style={{ width: "560px" }}>
        <div className="category-rule-edit-form">
          <div className="template-edit-grid">
            <Field label="名称">
              <input value={editing.name} onChange={(e) => setEditing({ ...editing, name: e.target.value })} placeholder="例如：GitHub 大文件" />
            </Field>
            <Field label="域名匹配模式">
              <input value={editing.domain_pattern} onChange={(e) => setEditing({ ...editing, domain_pattern: e.target.value })} placeholder="github.com 或 *.github.com" />
            </Field>
            <Field label="优先级（数字越小越优先）">
              <input type="number" value={editing.priority} onChange={(e) => setEditing({ ...editing, priority: +e.target.value })} />
            </Field>
            <Field label="连接数（留空表示不覆盖；仅允许 1 / 2 / 4 / 8 / 16 / 32）">
              <Select
                value={editing.connections ?? ""}
                onChange={(val: any) => {
                  setEditing({ ...editing, connections: val === "" ? null : +val });
                }}
                options={[
                  { value: "", label: "不覆盖" },
                  { value: 1, label: "1 路（单连接）" },
                  { value: 2, label: "2 路" },
                  { value: 4, label: "4 路" },
                  { value: 8, label: "8 路" },
                  { value: 16, label: "16 路" },
                  { value: 32, label: "32 路" },
                ]}
                ariaLabel="连接数"
              />
            </Field>
            <Field label="单任务限速（KB/s，0 或留空表示不限速）">
              <input
                type="number"
                min="0"
                value={editing.speed_limit ? Math.round(editing.speed_limit / 1024) : 0}
                onChange={(e) => {
                  const v = +e.target.value;
                  setEditing({ ...editing, speed_limit: v > 0 ? v * 1024 : null });
                }}
              />
            </Field>
            <Field label="启用">
              <div style={{ display: "flex", alignItems: "center", height: "28px" }}>
                <input className="toggle" type="checkbox" checked={editing.enabled} onChange={(e) => setEditing({ ...editing, enabled: e.target.checked })} />
              </div>
            </Field>
            <Field className="wide" label="保存目录（留空表示不覆盖）">
              <div className="input-group">
                <input
                  value={editing.destination ?? ""}
                  onChange={(e) => setEditing({ ...editing, destination: e.target.value || null })}
                  placeholder="例如：D:\\Downloads\\GitHub"
                />
                <button className="input-button" onClick={async () => {
                  const picked = await pickPath({ directory: true, multiple: false, title: "选择保存目录" });
                  if (typeof picked === "string") setEditing({ ...editing, destination: picked });
                }}>选择目录</button>
              </div>
            </Field>
            <Field className="wide" label="完成后动作（留空表示不覆盖）">
              <CompletionActionEditor
                value={editing.completion_action ?? "none"}
                onChange={(a) => setEditing({ ...editing, completion_action: a === "none" ? null : a })}
                hidePowerOptions
              />
            </Field>
            <Field className="wide" label="请求头（每行一个，格式 Key: Value；留空表示不覆盖）">
              <textarea
                rows={3}
                value={headersText}
                onChange={(e) => setHeadersText(e.target.value)}
                placeholder={"Authorization: Bearer token\nUser-Agent: MaobuFetch"}
                style={{ width: "100%", fontFamily: "monospace" }}
              />
            </Field>
          </div>
          <div className="dialog-actions"><button onClick={() => setEditing(null)}>取消</button><button className="primary" onClick={() => void saveEdit()}>保存</button></div>
        </div>
      </Modal>
    )}
  </SettingsGroup>;
}

/**
 * Task 25: 设置页中的标签管理面板。
 * - 列出全部标签（按 name 升序）
 * - 每条标签行：颜色块 + 名称（可编辑）+ 颜色选择器 + 删除按钮
 * - 顶部"新建标签"行：名称输入 + 颜色选择器 + 添加按钮
 * - 删除标签前确认（防止误删，因为级联会影响所有任务）
 */
function TagManagementPanel({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [tags, setTags] = useState<Tag[]>([]);
  const [loading, setLoading] = useState(true);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState("#3B82F6");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingName, setEditingName] = useState("");
  const [editingColor, setEditingColor] = useState("#3B82F6");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const load = async () => {
    setLoading(true);
    try {
      setTags(await api.tagList());
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void load(); }, [notify]);

  const add = async () => {
    const name = newName.trim();
    if (!name) {
      notify("标签名称不能为空", "error");
      return;
    }
    if (!/^#[0-9A-Fa-f]{6}$/.test(newColor)) {
      notify("颜色格式必须为 #RRGGBB", "error");
      return;
    }
    try {
      await api.tagAdd({ id: newTagId(), name, color: newColor });
      setNewName("");
      setNewColor("#3B82F6");
      notify("标签已创建");
      await load();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const beginEdit = (tag: Tag) => {
    setEditingId(tag.id);
    setEditingName(tag.name);
    setEditingColor(tag.color);
  };

  const cancelEdit = () => {
    setEditingId(null);
    setEditingName("");
    setEditingColor("#3B82F6");
  };

  const saveEdit = async () => {
    if (!editingId) return;
    const name = editingName.trim();
    if (!name) {
      notify("标签名称不能为空", "error");
      return;
    }
    if (!/^#[0-9A-Fa-f]{6}$/.test(editingColor)) {
      notify("颜色格式必须为 #RRGGBB", "error");
      return;
    }
    try {
      await api.tagUpdate({ id: editingId, name, color: editingColor });
      cancelEdit();
      notify("标签已更新");
      await load();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const remove = async (id: string) => {
    try {
      await api.tagDelete(id);
      setConfirmDelete(null);
      notify("标签已删除");
      await load();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  return (
    <SettingsGroup title="标签管理">
      <p className="settings-note">标签用于对下载任务进行分类管理。删除标签只会解除关联，不会删除实际任务或文件。</p>
      <div className="settings-group-content">
        <div className="tag-management-add-row">
          <div className="tag-color-col">
            <input
              type="color"
              value={newColor}
              onChange={(e) => setNewColor(e.target.value.toUpperCase())}
              aria-label="新标签颜色"
              title="标签颜色"
            />
          </div>
          <input
            type="text"
            placeholder="新标签名称"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            maxLength={20}
            aria-label="新标签名称"
          />
          <span className="tag-color-hex">{newColor}</span>
          <button className="primary" onClick={() => void add()} disabled={!newName.trim()}>添加</button>
        </div>
        {loading ? (
          <div className="center-state"><LoaderCircle className="spin" /></div>
        ) : tags.length === 0 ? (
          <p className="muted">尚未创建任何标签。</p>
        ) : (
          <div className="tag-management-list">
            {tags.map((tag) => (
              <div key={tag.id} className="tag-management-row">
                {editingId === tag.id ? (
                  <>
                    <div className="tag-color-col">
                      <input
                        type="color"
                        value={editingColor}
                        onChange={(e) => setEditingColor(e.target.value.toUpperCase())}
                        aria-label="编辑标签颜色"
                      />
                    </div>
                    <input
                      type="text"
                      value={editingName}
                      onChange={(e) => setEditingName(e.target.value)}
                      maxLength={20}
                      aria-label="编辑标签名称"
                    />
                    <button className="primary" onClick={() => void saveEdit()}>保存</button>
                    <button className="secondary-btn" onClick={() => cancelEdit()}>取消</button>
                  </>
                ) : (
                  <>
                    <div className="tag-color-col">
                      <span className="tag-swatch" style={{ background: tag.color }} aria-hidden="true" />
                    </div>
                    <span className="tag-name" title={tag.name}>{tag.name}</span>
                    <span className="tag-color-hex muted">{tag.color}</span>
                    <button className="secondary-btn" onClick={() => beginEdit(tag)} title="编辑">编辑</button>
                    <button className="danger-action" onClick={() => setConfirmDelete(tag.id)} title="删除">删除</button>
                  </>
                )}
              </div>
            ))}
          </div>
        )}
        {confirmDelete !== null && (
          <Modal title="删除标签" onClose={() => setConfirmDelete(null)} style={{ width: "360px" }}>
            <p style={{ fontSize: "11.5px", color: "var(--muted)", margin: "0 0 16px", lineHeight: "1.5" }}>
              确定要删除此标签吗？这仅会移除所有任务与该标签的关联，不会删除下载文件。
            </p>
            <div className="dialog-actions">
              <button onClick={() => setConfirmDelete(null)}>取消</button>
              <button className="danger" onClick={() => void remove(confirmDelete)}>删除</button>
            </div>
          </Modal>
        )}
      </div>
    </SettingsGroup>
  );
}
function MediaCredentialsGuideModal({ onClose }: { onClose: () => void }) {
  return (
    <Modal title="凭证获取指引与平台关键 Key 说明" onClose={onClose} style={{ width: "620px" }}>
      <div className="media-cred-guide-container">
        <div className="media-cred-guide-section">
          <h3>📌 如何通过浏览器开发者工具 (F12) 获取凭证</h3>
          <ol className="media-cred-guide-steps">
            <li>在 <b>Chrome / Edge</b> 浏览器中打开目标网站（如 B站、抖音、Twitter、YouTube）并登录您的账号。</li>
            <li>按键盘 <kbd>F12</kbd> 打开<b>开发者工具</b>，切换到 <b>Network (网络)</b> 标签页。</li>
            <li><b>刷新页面</b>或播放网页视频，在左侧请求列表中选中顶部任意主页面请求或 API 请求。</li>
            <li>在右侧 <b>Headers (请求头)</b> 区域找到 <code>Cookie</code>、<code>Referer</code> 和 <code>User-Agent</code> 字段。右键复制其完整值并粘贴至本软件。</li>
          </ol>
        </div>

        <div className="media-cred-guide-section">
          <h3>🔑 四大平台核心凭证 Key 校验指南</h3>
          <div className="media-cred-guide-platforms">
            <div className="platform-guide-card">
              <h4>哔哩哔哩 <code>bilibili.com</code></h4>
              <p>必须包含 <code>SESSDATA</code>、<code>bili_jct</code>、<code>DedeUserID</code></p>
              <span className="tip">支持获取 1080P+ 高清画质及大会员专属音轨。</span>
            </div>
            <div className="platform-guide-card">
              <h4>抖音 <code>douyin.com</code></h4>
              <p>必须包含 <code>sessionid</code> (或 <code>sessionid_ss</code>)、<code>passport_csrf_token</code>、<code>ttwid</code></p>
              <span className="tip">支持获取 4K/2K 无水印视频及高级高清源。</span>
            </div>
            <div className="platform-guide-card">
              <h4>Twitter / X <code>twitter.com / x.com</code></h4>
              <p>必须包含 <code>auth_token</code> 和 <code>ct0</code></p>
              <span className="tip"><code>ct0</code> 用于 x-csrf-token 鉴权，缺一不可。</span>
            </div>
            <div className="platform-guide-card">
              <h4>YouTube <code>youtube.com</code></h4>
              <p>必须包含 <code>LOGIN_INFO</code>、<code>SID</code>、<code>HSID</code>、<code>SSID</code>、<code>APISID</code>、<code>SAPISID</code></p>
              <span className="tip">用于突破年龄限制及会员专属视频下载。</span>
            </div>
          </div>
        </div>

        <div className="media-cred-guide-section">
          <h3>🧩 扩展程序自动同步说明</h3>
          <p className="settings-note" style={{ margin: 0 }}>
            若您已安装并配对<b>猫步下载器浏览器扩展</b>，在浏览器访问目标网页时，扩展也可自动捕获当前页面的凭证进行透传，无需频繁手动复制。
          </p>
        </div>

        <div className="dialog-actions">
          <button className="primary" onClick={onClose}>知道了</button>
        </div>
      </div>
    </Modal>
  );
}

/**
 * Task 46：媒体凭证管理面板。
 *
 * - 列出全部已存储的凭证（按域名升序）
 * - 每条凭证行：域名 + 更新时间 + 编辑/删除按钮
 * - 编辑对话框：Cookie（多行）/Referer/User-Agent，留空表示清除该字段
 * - 顶部"新增凭证"按钮：输入域名后保存
 * - Cookie 字段在数据库中以 DPAPI 密文存储，前端始终处理明文
 *
 * 安全约束：Cookie 不会写入日志或错误历史；面板上展示 Cookie 内容时使用
 * 等宽字体并在编辑对话框中以多行 textarea 形式呈现，便于用户核对。
 */
function MediaCredentialsPanel({ notify }: { notify: (text: string, kind?: "ok" | "error") => void }) {
  const [credentials, setCredentials] = useState<MediaCredential[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<MediaCredential | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [guideOpen, setGuideOpen] = useState(false);

  const [checkingDomains, setCheckingDomains] = useState<Record<string, boolean>>({});
  const [checkResults, setCheckResults] = useState<Record<string, MediaCredentialCheckResult>>({});
  const [checkingAll, setCheckingAll] = useState(false);

  const [editingCheckResult, setEditingCheckResult] = useState<MediaCredentialCheckResult | null>(null);
  const [editingChecking, setEditingChecking] = useState(false);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.mediaCredentialList();
      setCredentials(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { void reload(); }, []);

  const startAdd = () => {
    setEditing({ domain: "", cookie: "", referer: null, user_agent: null, updated_at: "" });
    setIsNew(true);
    setEditingCheckResult(null);
  };

  const startEdit = (cred: MediaCredential) => {
    setEditing({ ...cred });
    setIsNew(false);
    setEditingCheckResult(null);
  };

  const checkCredential = async (domain: string) => {
    setCheckingDomains((prev) => ({ ...prev, [domain]: true }));
    try {
      const res = await api.mediaCredentialCheck(domain);
      setCheckResults((prev) => ({ ...prev, [domain]: res }));
      if (res.valid) {
        notify(res.message);
      } else {
        notify(res.message, "error");
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setCheckingDomains((prev) => ({ ...prev, [domain]: false }));
    }
  };

  const checkAllCredentials = async () => {
    if (credentials.length === 0) return;
    setCheckingAll(true);
    notify("开始在线检测已保存的媒体凭证...");
    try {
      await Promise.all(
        credentials.map(async (c) => {
          setCheckingDomains((prev) => ({ ...prev, [c.domain]: true }));
          try {
            const res = await api.mediaCredentialCheck(c.domain);
            setCheckResults((prev) => ({ ...prev, [c.domain]: res }));
          } catch {
            // 忽略单个网络错误，保留在结果集
          } finally {
            setCheckingDomains((prev) => ({ ...prev, [c.domain]: false }));
          }
        })
      );
      notify("所有媒体凭证检测完毕");
    } finally {
      setCheckingAll(false);
    }
  };

  const checkEditingCredential = async () => {
    if (!editing) return;
    const domain = editing.domain.trim();
    if (!domain) {
      notify("域名不能为空", "error");
      return;
    }
    setEditingChecking(true);
    setEditingCheckResult(null);
    try {
      const res = await invoke<MediaCredentialCheckResult>("media_credential_check", { domain });
      setEditingCheckResult(res);
      if (res.valid) {
        notify(res.message);
      } else {
        notify(res.message, "error");
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setEditingChecking(false);
    }
  };

  const onDomainChange = (val: string) => {
    if (!editing) return;
    const next = { ...editing, domain: val };
    // 域名输入时自动建议默认 Referer (若目前为空)
    const lower = val.trim().toLowerCase();
    if (!editing.referer) {
      if (lower.includes("bilibili.com")) next.referer = "https://www.bilibili.com/";
      else if (lower.includes("douyin.com")) next.referer = "https://www.douyin.com/";
      else if (lower.includes("twitter.com") || lower.includes("x.com")) next.referer = "https://x.com/";
      else if (lower.includes("youtube.com")) next.referer = "https://www.youtube.com/";
    }
    setEditing(next);
  };

  const saveEdit = async () => {
    if (!editing) return;
    const domain = editing.domain.trim();
    if (!domain) {
      notify("域名不能为空", "error");
      return;
    }
    // 简单校验域名格式：不允许含空格、协议前缀或路径分隔符
    if (/\s/.test(domain) || /^https?:\/\//i.test(domain) || /[\/\\]/.test(domain)) {
      notify("域名格式无效（应为裸域名，如 example.com）", "error");
      return;
    }
    const toSave: MediaCredential = {
      domain,
      cookie: editing.cookie?.trim() ?? "",
      referer: editing.referer?.trim() ? editing.referer!.trim() : null,
      user_agent: editing.user_agent?.trim() ? editing.user_agent!.trim() : null,
      updated_at: new Date().toISOString(),
    };
    try {
      await api.mediaCredentialSave(toSave);
      notify(isNew ? "已保存媒体凭证" : "已更新媒体凭证");
      setEditing(null);
      await reload();
      // 自动触发一次检测，增强反馈
      void checkCredential(domain);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeCredential = async (domain: string) => {
    if (!confirm(`确定删除域名 ${domain} 的凭证？`)) return;
    try {
      await api.mediaCredentialDelete(domain);
      notify("已删除媒体凭证");
      setCheckResults((prev) => {
        const next = { ...prev };
        delete next[domain];
        return next;
      });
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const fmtUpdated = (v?: string): string => {
    if (!v) return "—";
    try {
      const d = new Date(v);
      if (Number.isNaN(d.getTime())) return v;
      return d.toLocaleString();
    } catch {
      return v;
    }
  };

  const maskCookie = (cookie?: string): string => {
    if (!cookie) return "—";
    if (cookie.length <= 12) return "已保存（较短）";
    return `已保存（${cookie.length} 字符）`;
  };

  return <SettingsGroup title="媒体凭证管理">
    <p className="settings-note">
      按域名保存 Cookie 和 Referer 等凭证以在下载时自动附带。Cookie 使用 Windows DPAPI 加密存储。深度支持 Bilibili / 抖音 / Twitter(X) / YouTube 在线有效性检测。
    </p>
    <div className="category-rules-toolbar">
      <button className="input-button" onClick={startAdd}><Plus size={13} /><span>新增凭证</span></button>
      <button className="input-button" onClick={() => setGuideOpen(true)}><HelpCircle size={13} /><span>凭证获取指引</span></button>
      <button className="input-button" onClick={() => void reload()}><RefreshCw size={13} /><span>刷新</span></button>
      <button
        className="input-button"
        disabled={credentials.length === 0 || checkingAll}
        onClick={() => void checkAllCredentials()}
      >
        {checkingAll ? <LoaderCircle size={13} className="spin" /> : <ShieldCheck size={13} />}
        <span>{checkingAll ? "检测中..." : "批量检测"}</span>
      </button>
    </div>
    {loading ? <LoaderCircle className="spin" /> : credentials.length === 0 ? <p className="settings-note">暂无已保存的媒体凭证。</p> : (
      <div className="category-rules-list" role="table">
        <div className="category-rule-row media-credential-row category-rule-row-header" role="row">
          <span className="category-rule-name">域名</span>
          <span className="category-rule-pattern">Cookie</span>
          <span>Referer</span>
          <span>User-Agent</span>
          <span>更新时间</span>
          <span className="category-rule-actions">操作</span>
        </div>
        {credentials.map((cred) => {
          const res = checkResults[cred.domain];
          const isChecking = !!checkingDomains[cred.domain];
          return (
            <div key={cred.domain} style={{ display: "flex", flexDirection: "column" }}>
              <div className="category-rule-row media-credential-row" role="row">
                <span className="category-rule-name" role="cell" title={cred.domain}><code>{cred.domain}</code></span>
                <span className="category-rule-pattern" role="cell">{maskCookie(cred.cookie)}</span>
                <span role="cell" title={cred.referer ?? ""}>{cred.referer ? "已设置" : "—"}</span>
                <span role="cell" title={cred.user_agent ?? ""}>{cred.user_agent ? "已设置" : "—"}</span>
                <span role="cell">{fmtUpdated(cred.updated_at)}</span>
                <span className="category-rule-actions" role="cell">
                  <button
                    title="在线检测凭证有效性"
                    disabled={isChecking}
                    onClick={() => void checkCredential(cred.domain)}
                  >
                    {isChecking ? <LoaderCircle size={12} className="spin" /> : <ShieldCheck size={12} />}
                    <span>检测</span>
                  </button>
                  <button title="编辑" onClick={() => startEdit(cred)}>编辑</button>
                  <button title="删除" className="danger" onClick={() => void removeCredential(cred.domain)}><Trash2 size={12} /></button>
                </span>
              </div>
              {res && (
                <div className={`media-cred-result-box ${res.valid ? "valid" : "invalid"}`}>
                  {res.valid ? <CheckCircle2 size={13} /> : <AlertCircle size={13} />}
                  <span>{res.message}</span>
                </div>
              )}
            </div>
          );
        })}
      </div>
    )}
    {guideOpen && <MediaCredentialsGuideModal onClose={() => setGuideOpen(false)} />}
    {editing && (
      <Modal title={isNew ? "新增媒体凭证" : "编辑媒体凭证"} onClose={() => setEditing(null)} style={{ width: "560px" }}>
        <div className="category-rule-edit-form">
          <Field label="域名">
            <input
              value={editing.domain}
              onChange={(e) => onDomainChange(e.target.value)}
              placeholder="如 bilibili.com (裸域名，不含 http:// 或 https:// 协议前缀)"
              disabled={!isNew}
            />
          </Field>
          <Field label="Cookie">
            <textarea
              rows={5}
              value={editing.cookie ?? ""}
              onChange={(e) => setEditing({ ...editing, cookie: e.target.value })}
              placeholder="输入或粘贴 Cookie 键值对内容 (多行 name=value 形式，留空表示清除)"
              style={{ width: "100%", fontFamily: "monospace" }}
            />
          </Field>
          <Field label="Referer">
            <input
              value={editing.referer ?? ""}
              onChange={(e) => setEditing({ ...editing, referer: e.target.value || null })}
              placeholder="https://example.com/ (选填，留空表示不设置)"
            />
          </Field>
          <Field label="User-Agent">
            <input
              value={editing.user_agent ?? ""}
              onChange={(e) => setEditing({ ...editing, user_agent: e.target.value || null })}
              placeholder="Mozilla/5.0 ... (选填，使用自定义 User-Agent；留空表示使用软件默认)"
            />
            <button
              type="button"
              className="input-button"
              style={{ fontSize: "11px", padding: "2px 8px", alignSelf: "flex-start", marginTop: "2px" }}
              onClick={() => setEditing({ ...editing, user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36" })}
            >
              <span>填充默认 UA</span>
            </button>
          </Field>

          <details className="category-rule-test-details" style={{ marginTop: "4px" }}>
            <summary><Info size={12} /><span>如何获取 Cookie 与四大平台关键 Key 提示</span></summary>
            <div className="category-rule-test-body" style={{ fontSize: "11px", lineHeight: "1.5" }}>
              <p style={{ margin: 0 }}><b>F12 抓包方法</b>：打开 Chrome/Edge → 登录网站 → F12 → Network 页签 → 刷新页面点任意请求 → 复制 Headers 里的 Cookie 值。</p>
              {editing.domain.includes("bilibili") && <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>💡 <b>B站关键 Key</b>：请确保包含 <code>SESSDATA</code>、<code>bili_jct</code>、<code>DedeUserID</code>。</p>}
              {editing.domain.includes("douyin") && <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>💡 <b>抖音关键 Key</b>：请确保包含 <code>sessionid</code> (或 <code>sessionid_ss</code>) 和 <code>ttwid</code>。</p>}
              {(editing.domain.includes("twitter") || editing.domain.includes("x.com")) && <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>💡 <b>Twitter/X 关键 Key</b>：请确保包含 <code>auth_token</code> 和 <code>ct0</code>。</p>}
              {editing.domain.includes("youtube") && <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>💡 <b>YouTube 关键 Key</b>：请确保包含 <code>LOGIN_INFO</code> 和 <code>SID</code>。</p>}
            </div>
          </details>

          {editingCheckResult && (
            <div className={`media-cred-result-box ${editingCheckResult.valid ? "valid" : "invalid"}`}>
              {editingCheckResult.valid ? <CheckCircle2 size={13} /> : <AlertCircle size={13} />}
              <span>{editingCheckResult.message}</span>
            </div>
          )}
          <div className="dialog-actions">
            <button
              type="button"
              disabled={editingChecking || !editing.domain.trim()}
              onClick={() => void checkEditingCredential()}
            >
              {editingChecking ? <LoaderCircle size={13} className="spin" /> : <ShieldCheck size={13} />}
              <span>检测凭证</span>
            </button>
            <button onClick={() => setEditing(null)}>取消</button>
            <button className="primary" onClick={() => void saveEdit()}>保存</button>
          </div>
        </div>
      </Modal>
    )}
  </SettingsGroup>;
}

function ContextMenu({ x, y, task, selectedTaskIds, allTasks = [], close, notify, onSetSpeedLimit, onDelete, onViewDetails }: { x: number; y: number; task: DownloadTask; selectedTaskIds?: Set<string>; allTasks?: DownloadTask[]; close: () => void; notify: (text: string, kind?: "ok" | "error") => void; onSetSpeedLimit: (task: DownloadTask) => void; onDelete: (taskIds: Set<string>, deleteFile: boolean) => void; onViewDetails?: () => void }) {
  const targetTaskIds = useMemo(() => {
    if (selectedTaskIds && selectedTaskIds.has(task.id) && selectedTaskIds.size > 1) {
      return selectedTaskIds;
    }
    return new Set([task.id]);
  }, [selectedTaskIds, task.id]);

  const targetTasks = useMemo(() => {
    return allTasks.filter((t) => targetTaskIds.has(t.id));
  }, [allTasks, targetTaskIds]);

  const countTag = targetTaskIds.size > 1 ? ` (${targetTaskIds.size} 项)` : "";

  const action = async (value: string) => {
    try {
      for (const id of targetTaskIds) {
        await api.action(id, value);
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      close();
    }
  };
  const update = async (options: { priority?: number; perTaskSpeedLimit?: number; completionAction?: CompletionAction }) => {
    try {
      for (const id of targetTaskIds) {
        await api.updateTaskOptions(id, options);
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      close();
    }
  };
  const changeSpeedLimit = () => {
    onSetSpeedLimit(task);
    close();
  };
  const copyText = async (label: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      notify(`${label}已复制`);
    } catch (error) {
      notify(`复制${label}失败：${String(error)}`, "error");
    } finally {
      close();
    }
  };
  const copyUrls = async () => {
    try {
      const list = targetTasks.length > 0 ? targetTasks : [task];
      const text = list.map((t) => t.url).join("\n");
      await navigator.clipboard.writeText(text);
      notify(list.length > 1 ? `${list.length} 个来源链接已复制` : "来源链接已复制");
    } catch (error) {
      notify(`复制链接失败：${String(error)}`, "error");
    } finally {
      close();
    }
  };
  const buildFilePath = () => {
    const sep = task.destination.endsWith("\\") || task.destination.endsWith("/") ? "" : "\\";
    return `${task.destination}${sep}${task.file_name}`;
  };
  const showDiagnosis = () => {
    const detail = task.error || "未记录详细错误信息。可尝试重试，若仍失败请检查链接、登录态或网络。";
    notify(`诊断：${detail}`, "error");
    close();
  };
  const confirmDelete = (deleteFile: boolean) => {
    onDelete(targetTaskIds, deleteFile);
    close();
  };

  const menuWidth = 220;
  const itemHeight = 30;
  const separatorHeight = 9;
  const padding = 8;

  const sections: ReactNode[] = [];
  const pushSep = () => sections.push(<div key={`sep-${sections.length}`} className="context-menu-separator" />);

  switch (task.status) {
    case "downloading":
    case "verifying":
    case "waiting-network":
      sections.push(<button key="pause" onClick={() => void action("pause")}><Pause size={13} />暂停</button>);
      break;
    case "paused":
      sections.push(<button key="resume" onClick={() => void action("resume")}><Play size={13} />继续</button>);
      break;
    case "interrupted":
      sections.push(<button key="resume" onClick={() => void action("resume")}><Play size={13} />继续</button>);
      break;
    case "paused-by-low-disk":
      sections.push(<button key="resume" onClick={() => void action("resume")}><Play size={13} />继续</button>);
      sections.push(<button key="change-dir" disabled title="请在设置中更换默认下载目录后继续"><FolderOpen size={13} />更换目录</button>);
      break;
    case "paused-by-metered":
      sections.push(<button key="resume" onClick={() => void action("resume")}><Play size={13} />继续下载</button>);
      break;
    case "failed":
      sections.push(<button key="diagnose" onClick={() => showDiagnosis()}><AlertCircle size={13} />诊断错误</button>);
      sections.push(<button key="retry" onClick={() => void action("retry")}><RefreshCw size={13} />重试</button>);
      break;
    case "remote-changed":
      sections.push(<button key="redownload" onClick={() => void action("redownload")}><RefreshCw size={13} />重新下载</button>);
      sections.push(<button key="keep-cancel" onClick={() => void action("cancel")}><CirclePause size={13} />保留旧文件并取消</button>);
      break;
    case "completed":
      if (targetTaskIds.size <= 1) {
        sections.push(<button key="open-file" onClick={() => void api.openFile(task.id).then(close).catch((e) => notify(String(e), "error"))}><ExternalLink size={13} />打开文件</button>);
        sections.push(<button key="open-folder" onClick={() => void api.openFolder(task.id).then(close).catch((e) => notify(String(e), "error"))}><FolderOpen size={13} />打开文件夹</button>);
      }
      sections.push(<button key="copy-path" onClick={() => {
        if (targetTaskIds.size <= 1) {
          void copyText("文件路径", buildFilePath());
        } else {
          const paths = targetTasks.map((t) => {
            const sep = t.destination.endsWith("\\") || t.destination.endsWith("/") ? "" : "\\";
            return `${t.destination}${sep}${t.file_name}`;
          }).join("\n");
          void copyText("文件路径", paths);
        }
      }}><Copy size={13} />复制文件路径{countTag}</button>);
      if (targetTaskIds.size <= 1) {
        sections.push(<button key="verify" onClick={() => void api.verify(task.id).then(() => { notify("文件校验完成"); close(); }).catch((e) => notify(String(e), "error"))}><ShieldCheck size={13} />校验 SHA-256</button>);
      }
      sections.push(<button key="redownload" onClick={() => void action("redownload")}><RefreshCw size={13} />重新下载{countTag}</button>);
      break;
    case "queued":
    case "scheduled":
    case "cancelled":
    default:
      break;
  }

  if (!["cancelled", "completed"].includes(task.status)) {
    sections.push(
      <div key="priority-row" className="context-menu-row-item">
        <span className="context-menu-row-label">队列顺序</span>
        <div className="context-menu-row-buttons">
          <button onClick={() => void update({ priority: MIN_PRIORITY })} title="置顶"><ChevronsUp size={13} /></button>
          <button onClick={() => void update({ priority: clampPriority(task.priority - PRIORITY_STEP) })} title="上移"><ChevronUp size={13} /></button>
          <button onClick={() => void update({ priority: clampPriority(task.priority + PRIORITY_STEP) })} title="下移"><ChevronDown size={13} /></button>
          <button onClick={() => void update({ priority: MAX_PRIORITY })} title="置底"><ChevronsDown size={13} /></button>
        </div>
      </div>
    );
    sections.push(<button key="speed-limit" onClick={() => void changeSpeedLimit()}><Gauge size={13} />任务限速：{task.per_task_speed_limit ? `${Math.round(task.per_task_speed_limit / 1024)} KB/s` : "不限速"}</button>);
    sections.push(<button key="completion" onClick={() => void update({ completionAction: task.completion_action === "open-folder" ? "none" : "open-folder" })}><FolderOpen size={13} />{task.completion_action === "open-folder" ? "取消完成后打开文件夹" : "完成后打开文件夹"}</button>);
  }

  if (onViewDetails) {
    sections.push(<button key="view-details" onClick={() => { onViewDetails(); close(); }}><Info size={13} />查看详情</button>);
  }

  if (sections.length > 0) pushSep();
  sections.push(<button key="copy-url" onClick={() => void copyUrls()}><Copy size={13} />复制链接{countTag}</button>);

  pushSep();
  sections.push(
    <button key="delete-record" className="danger" onClick={() => void confirmDelete(false)}>
      <Trash2 size={13} />
      {t("dialogs.deleteRecordOnly")}{countTag}
    </button>
  );
  sections.push(
    <button key="delete-file" className="danger" onClick={() => void confirmDelete(true)}>
      <Trash2 size={13} />
      {t("dialogs.deleteRecordAndFile")}{countTag}
    </button>
  );

  let separatorCount = 0;
  for (const node of sections) {
    if ((node as any)?.props?.className === "context-menu-separator") separatorCount++;
  }
  const buttonCount = sections.length - separatorCount;
  const menuHeight = buttonCount * itemHeight + separatorCount * separatorHeight + padding;
  const safeX = Math.max(8, Math.min(x, window.innerWidth - menuWidth - 8));
  const safeY = Math.max(8, Math.min(y, window.innerHeight - menuHeight - 8));

  return (
    <div className="context-menu" style={{ left: safeX, top: safeY, minWidth: menuWidth }} onClick={(e) => e.stopPropagation()}>
      {sections}
    </div>
  );
}



/** Task 21.2：重命名任务文件名对话框。仅 Queued 状态可调用。
 * 前端做基础校验（非空、无非法字符），后端再做重名/状态校验。 */
function RenameDialog({ task, onClose, onRenamed }: { task: DownloadTask; onClose: () => void; onRenamed: (newName: string) => void }) {
  // Task 33: 订阅 locale 变化，对话框文案同步刷新。
  useLocale();
  const [value, setValue] = useState(task.file_name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const inputRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    inputRef.current?.focus();
    // 选中文件名主体（不含扩展名），符合 Windows 资源管理器 F2 行为
    const dot = task.file_name.lastIndexOf(".");
    inputRef.current?.setSelectionRange(0, dot > 0 ? dot : task.file_name.length);
  }, [task.file_name]);
  const submit = async () => {
    const trimmed = value.trim();
    if (!trimmed) { setError(t("dialogs.renamePlaceholder")); return; }
    if (trimmed === task.file_name) { onClose(); return; }
    if (/[<>:"/\\|?*]/.test(trimmed) || /[\x00-\x1f]/.test(trimmed)) {
      setError(`${trimmed}`);
      return;
    }
    if (trimmed.includes("..") || trimmed.startsWith("/") || trimmed.startsWith("\\")) {
      setError(`${trimmed}`);
      return;
    }
    if (trimmed.length > 255) { setError(`${trimmed}`); return; }
    setBusy(true);
    setError(undefined);
    try {
      await api.rename(task.id, trimmed);
      onRenamed(trimmed);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title={t("dialogs.renameTitle")} onClose={onClose} style={{ width: "420px" }}>
      <div className="delete-task-dialog">
        <p className="delete-task-message">
          {t("dialogs.renameTitle")}：<strong title={task.file_name}>{task.file_name}</strong>
        </p>
        <label className="form-field">
          <span>{t("dialogs.renamePlaceholder")}</span>
          <input
            ref={inputRef}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void submit(); }
              if (e.key === "Escape") { e.preventDefault(); onClose(); }
            }}
            disabled={busy}
            placeholder={t("dialogs.renamePlaceholder")}
          />
        </label>
        {error && <p className="inline-error">{error}</p>}
        <div className="dialog-actions">
          <button onClick={onClose} disabled={busy}>{t("common.cancel")}</button>
          <button className="primary" onClick={() => void submit()} disabled={busy || !value.trim()}>{t("common.rename")}</button>
        </div>
      </div>
    </Modal>
  );
}



function SpeedLimitDialog({
  task,
  onClose,
  onConfirm,
}: {
  task: DownloadTask;
  onClose: () => void;
  onConfirm: (limitKb: number) => Promise<void>;
}) {
  useLocale();
  const currentLimit = Math.round(task.per_task_speed_limit / 1024);
  const [value, setValue] = useState(String(currentLimit));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const submit = async () => {
    const limit = Number(value);
    if (!Number.isFinite(limit) || limit < 0) {
      setError("请输入不小于 0 的有效数字");
      return;
    }
    setBusy(true);
    try {
      await onConfirm(limit);
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title="任务限速" onClose={onClose} style={{ width: "420px" }}>
      <div className="delete-task-dialog">
        <p className="delete-task-message">
          任务限速：<strong title={task.file_name}>{task.file_name}</strong>
        </p>
        <label className="form-field">
          <span>限速值 (KB/s，0 表示不限速)</span>
          <input
            ref={inputRef}
            type="number"
            min="0"
            step="1"
            value={value}
            onChange={(e) => {
              setValue(e.target.value);
              setError(undefined);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void submit(); }
              if (e.key === "Escape") { e.preventDefault(); onClose(); }
            }}
            placeholder="0"
            disabled={busy}
          />
        </label>
        {error && <p className="inline-error">{error}</p>}
        <div className="dialog-actions">
          <button onClick={onClose} disabled={busy}>{t("common.cancel")}</button>
          <button className="primary" onClick={() => void submit()} disabled={busy}>{t("common.ok")}</button>
        </div>
      </div>
    </Modal>
  );
}



export function Modal({ title, onClose, wide, children, style, headerAction }: { title: string; onClose: () => void; wide?: boolean; children: ReactNode; style?: CSSProperties; headerAction?: ReactNode }) {
  return (
    <div className="modal-layer" onMouseDown={onClose}>
      <div className="dialog-material" onMouseDown={(e) => e.stopPropagation()}>
        <section className={wide ? "dialog wide" : "dialog"} style={style}>
          <div className="settings-title">
            <h2>{title}</h2>
            {headerAction}
          </div>
          {children}
        </section>
      </div>
    </div>
  );
}

/** Task 27.6：备份选项对话框。
 *
 * 让用户选择是否包含认证信息（Cookie/Authorization/代理密码）。
 * 勾选后必须输入密码，备份文件将以 AES-256-GCM 加密。 */
function formatBytes(value: number) { if (!value) return "0 B"; const units = ["B","KB","MB","GB","TB"]; const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1); return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`; }
function formatDuration(seconds: number) { if (seconds < 60) return `${seconds} 秒`; if (seconds < 3600) return `${Math.ceil(seconds / 60)} 分钟`; return `${Math.floor(seconds / 3600)} 小时 ${Math.ceil(seconds % 3600 / 60)} 分`; }
function formatDate(value: number) { return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(new Date(value)); }

/** 队列调度可观察性（Task 15）：将等待原因枚举映射为简体中文说明。NotWaiting 返回 null。 */
function waitReasonText(reason: WaitReason): string | null {
  switch (reason.kind) {
    case "not-waiting": return null;
    case "queued-behind": return `等待前面 ${reason.ahead_count} 个任务完成`;
    case "waiting-media-tools": return "等待媒体工具安装";
    case "waiting-user-confirmation": return "等待用户确认";
    case "waiting-scheduled-time": return `等待计划时间：${formatScheduleTime(reason.scheduled_at)}`;
    case "waiting-concurrency-limit": return `等待并发槽位（当前已满 ${reason.active_count} 个）`;
    case "paused": return "用户已暂停";
    case "paused-by-low-disk": return "磁盘空间不足已暂停";
    case "paused-by-metered": return "计量网络下已自动暂停";
    case "interrupted": return "任务已中断，可继续";
    case "remote-changed": return "远端资源已变化";
    case "unknown": return "未知状态";
  }
}
/** 格式化计划时间戳字符串（Unix 毫秒）为本地时间 "YYYY/MM/DD HH:MM"。 */
function formatScheduleTime(epochMsStr: string): string {
  const ms = Number(epochMsStr);
  if (!ms || !Number.isFinite(ms)) return "—";
  return new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(new Date(ms));
}
function redactedUrl(value: string) { try { const url = new URL(value); url.username = ""; url.password = ""; url.search = ""; url.hash = ""; return url.toString(); } catch { return "地址格式无效"; } }
function hostOf(url: string) { try { return new URL(url).host; } catch { return url; } }
function safeDisplayName(value: string) { return value.replace(/[<>:"/\\|?*]/g, "_").slice(0, 120); }

/**
 * Task 25: 判断任务是否匹配高级筛选条件。空条件表示不限制该维度。
 *
 * - `statuses`: 空数组 = 不过滤；非空 = 任务状态必须命中其一
 * - `domain`: 空字符串 = 不过滤；非空 = URL host 包含该子串（大小写不敏感）
 * - `dateFrom` / `dateTo`: Unix 毫秒；任务 created_at 必须落在区间内（闭区间）
 * - `sizeMin` / `sizeMax`: 字节；任务 total_bytes 必须落在区间内（闭区间）
 * - `tagIds`: 空数组 = 不过滤；非空 = 任务的标签 id 集合必须包含全部所选 tag
 *   （AND 语义，避免空集假阳性命中）
 * - `sources`: 空数组 = 不过滤；非空 = 任务 source 字段必须命中其一
 */
function matchesAdvancedFilter(task: DownloadTask, filter: AdvancedFilter, taskTagList: Tag[]): boolean {
  if (filter.statuses.length > 0 && !filter.statuses.includes(task.status)) return false;
  if (filter.domain.trim()) {
    const host = hostOf(task.url).toLowerCase();
    if (!host.includes(filter.domain.trim().toLowerCase())) return false;
  }
  if (filter.dateFrom != null && task.created_at < filter.dateFrom) return false;
  if (filter.dateTo != null && task.created_at > filter.dateTo) return false;
  if (filter.sizeMin != null && (task.total_bytes ?? 0) < filter.sizeMin) return false;
  if (filter.sizeMax != null && (task.total_bytes ?? 0) > filter.sizeMax) return false;
  if (filter.tagIds.length > 0) {
    const taskTagIds = new Set(taskTagList.map((t) => t.id));
    for (const required of filter.tagIds) {
      if (!taskTagIds.has(required)) return false;
    }
  }
  if (filter.sources.length > 0 && !filter.sources.includes(task.source)) return false;
  return true;
}

/** Task 25: 判断筛选条件是否为空（不限制任何维度）。用于决定是否显示"清除筛选"按钮。 */
function isAdvancedFilterEmpty(filter: AdvancedFilter): boolean {
  return filter.statuses.length === 0
    && !filter.domain.trim()
    && filter.dateFrom == null
    && filter.dateTo == null
    && filter.sizeMin == null
    && filter.sizeMax == null
    && filter.tagIds.length === 0
    && filter.sources.length === 0;
}

/** Task 25: 生成新标签 ID（前端时间戳 + 随机数，避免引入 uuid 依赖）。 */
function newTagId(): string {
  return `tag-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Task 25: 生成新快捷视图 ID。 */
function newQuickViewId(): string {
  return `view-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function isDownloadableUrl(url: string): boolean {
  try {
    const trimmed = url.trim();
    if (!/^https?:\/\/[^\s]+$/i.test(trimmed)) {
      return false;
    }

    const parsed = new URL(trimmed);
    const pathname = parsed.pathname.toLowerCase();
    
    // 1. 纯根目录链接（例如 https://github.com/ 或 https://bilibili.com ），排除
    if (pathname === "/" || pathname === "") {
      const search = parsed.search.toLowerCase();
      if (search.includes("download") || search.includes("file=") || search.includes("url=")) {
        return true;
      }
      return false;
    }

    // 2. 普通 HTML/网页扩展名，排除
    const pageExtensions = [".html", ".htm", ".shtml", ".jsp", ".php", ".asp", ".aspx"];
    if (pageExtensions.some(ext => pathname.endsWith(ext))) {
      const search = parsed.search.toLowerCase();
      if (search.includes("download") || search.includes("file=") || search.includes("url=")) {
        return true;
      }
      return false;
    }

    // 3. 常见可下载文件扩展名，放行
    const downloadExtensions = [
      ".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz", ".pkg", ".dmg", ".iso", ".tgz",
      ".exe", ".msi", ".apk", ".ipa", ".deb", ".rpm",
      ".mp4", ".mkv", ".avi", ".mov", ".wmv", ".flv", ".webm", ".m3u8", ".ts", ".rmvb",
      ".mp3", ".flac", ".wav", ".aac", ".ogg", ".m4a", ".ape",
      ".pdf", ".epub", ".docx", ".xlsx", ".pptx", ".torrent"
    ];
    const lastSegment = pathname.split('/').pop() || "";
    if (downloadExtensions.some(ext => lastSegment.endsWith(ext))) {
      return true;
    }

    // 4. 音视频分享网站，放行
    const mediaDomains = [
      "youtube.com", "youtu.be", "bilibili.com", "b23.tv", "douyin.com", "iesdouyin.com", "douyinvod.com",
      "vimeo.com", "tiktok.com", "twitter.com", "x.com", "weibo.com"
    ];
    const hostname = parsed.hostname.toLowerCase();
    if (mediaDomains.some(domain => hostname === domain || hostname.endsWith("." + domain))) {
      return true;
    }

    // 5. URL 路径或参数包含下载敏感词，放行
    const downloadKeywords = ["/download", "/attachment", "/file/", "/release/", "/update/"];
    if (downloadKeywords.some(keyword => pathname.includes(keyword))) {
      return true;
    }
    const search = parsed.search.toLowerCase();
    if (search.includes("download") || search.includes("file=") || search.includes("url=")) {
      return true;
    }

    return false;
  } catch {
    return false;
  }
}

/**
 * Task 46：从 URL 提取注册域名（去掉前导 `www.`），用于匹配已保存的媒体凭证。
 * 解析失败或非 http(s) 返回 null。与后端 `media_cookies::extract_domain` 保持一致。
 */
function extractDomainForHint(url: string): string | null {
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

function CloseConfirmDialog({ onClose, onConfirm }: { onClose: () => void; onConfirm: (action: "tray" | "exit", remember: boolean) => void }) {
  // Task 33: 订阅 locale 变化，对话框文案同步刷新。
  useLocale();
  const [remember, setRemember] = useState(false);

  return (
    <Modal title={t("dialogs.closeTitle")} onClose={onClose} style={{ width: "380px" }}>
      <div className="new-task-form" style={{ gap: "16px", padding: "4px 0 0" }}>
        <p style={{ margin: 0, fontSize: "12px", color: "var(--text)", lineHeight: 1.5 }}>
          {t("dialogs.closeQuestion")}
        </p>

        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginTop: "8px", width: "100%", gap: "12px" }}>
          <label style={{ display: "flex", alignItems: "center", gap: "6px", fontSize: "11px", color: "var(--muted)", cursor: "pointer", userSelect: "none" }}>
            <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} style={{ width: "13px", height: "13px", accentColor: "var(--accent)" }} />
            <span>{t("dialogs.rememberChoice")}</span>
          </label>

          <div style={{ display: "flex", gap: "8px" }}>
            <button
              className="dialog-actions-btn"
              style={{
                height: "28px",
                padding: "0 12px",
                borderRadius: "6px",
                border: "1px solid var(--border-strong)",
                background: "var(--control)",
                color: "var(--text)",
                fontSize: "11px",
                fontWeight: 500,
                cursor: "pointer"
              }}
              onClick={() => onConfirm("exit", remember)}
            >
              {t("dialogs.closeExit")}
            </button>
            <button
              className="dialog-actions-btn primary"
              style={{
                height: "28px",
                padding: "0 12px",
                borderRadius: "6px",
                border: "none",
                background: "var(--accent)",
                color: "white",
                fontSize: "11px",
                fontWeight: 500,
                cursor: "pointer"
              }}
              onClick={() => onConfirm("tray", remember)}
            >
              {t("dialogs.closeMinimize")}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
