import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { onAction } from "@tauri-apps/plugin-notification";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import {
  AlertCircle,
  AlertTriangle,
  Archive,
  Check,
  ChevronDown,
  Download,
  ExternalLink,
  FolderOpen,
  ListFilter,
  LoaderCircle,
  PanelRightClose,
  PanelRightOpen,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings,
  ShieldCheck,
  Trash2,
  Unplug,
  X,
} from "lucide-react";
import { api, isDesktop } from "./api";
import { setLocale, t, useLocale } from "./i18n";
import type {
  AdvancedFilter,
  AppSettings,
  DeepLinkReceivedPayload,
  DownloadTask,
  FilterKey,
  MeteredNetworkDetectedPayload,
  PowerAction,
  PowerActionState,
  QuickView,
  SelfcheckReport,
  Tag,
  TaskNotificationPayload,
  TaskStatus,
  TaskTagsMap,
} from "./types";
import { EMPTY_ADVANCED_FILTER } from "./types";
import {
  DEFAULT_SHORTCUTS,
  formatBytes,
  getCategories,
  getNav,
  isAdvancedFilterEmpty,
  isDownloadableUrl,
  matchesAdvancedFilter,
  matchesShortcut,
  newQuickViewId,
} from "./formatters";
import {
  defaultHistoryDateRange,
  HistoryDateFilter,
  matchesHistoryDate,
} from "./components/HistoryDateFilter";
import {
  PowerActionBanner,
  PowerActionButton,
} from "./components/PowerActionControl";
import { Select } from "./components/Select";
import { YouTubeCredentialsModal } from "./components/YouTubeCredentialsModal";
import {
  CatDownloadMark,
  EmptyState,
} from "./components/common/EmptyState";
import { Modal } from "./components/common/Modal";
import { TaskRow } from "./components/common/TaskRow";
import { BulkActionBar } from "./components/common/BulkActionBar";
import {
  applyWindowAppearance,
  Titlebar,
  usesDarkTheme,
  WindowResizeHandles,
} from "./components/common/Titlebar";
import { ContextMenu } from "./components/common/ContextMenu";
import { Details } from "./components/details/Details";
import { AdvancedFilterPanel } from "./components/modals/AdvancedFilterPanel";
import { CloseConfirmDialog } from "./components/modals/CloseConfirmDialog";
import { NewTaskDialog } from "./components/modals/NewTaskDialog";
import { RenameDialog } from "./components/modals/RenameDialog";
import { RefreshUrlDialog } from "./components/modals/RefreshUrlDialog";
import { SpeedLimitDialog } from "./components/modals/SpeedLimitDialog";
import { SettingsPage } from "./components/settings/SettingsPage";
import {
  arrayBufferToBase64,
  classifyDroppedFiles,
  extractDroppedUrls,
  MAX_DROPPED_TORRENT_BYTES,
} from "./drag-drop";
import { reorderTaskIdsWithinPriority } from "./priority";

const defaults: AppSettings = {
  download_dir: "",
  concurrent_downloads: 3,
  connections_per_download: 8,
  speed_limit_kbps: 0,
  start_minimized: false,
  minimize_to_tray: true,
  close_to_tray: false,
  notifications: true,
  auto_start: true,
  theme: "system",
  accent_color: "blue",
  frosted_glass: false,
  scheduled_limit: null,
  bt_extra_trackers: "",
  language: "zh-CN",
  intercept_browser_downloads: true,
  min_file_size_mb: 1,
  clipboard_monitor: false,
  proxy_mode: "system",
  proxy_url: "",
  proxy_username: "",
  proxy_password: "",
  user_agent: "MaobuFetch/0.5",
  default_collision_policy: "rename",
  default_completion_action: "none",
  max_retries: 3,
  retry_base_seconds: 2,
  verify_after_download: false,
  media_tool_auto_update: true,
  yt_dlp_path: "",
  ffmpeg_path: "",
  ffprobe_path: "",
  youtube_po_token: "",
  low_memory_mode: false,
  window_width: 1024,
  window_height: 720,
  auto_scale_ui: false,
  default_retry_policy: {
    connection_timeout_secs: 60,
    task_timeout_secs: null,
    max_retries: 5,
    backoff: "exponential",
    initial_backoff_ms: 1000,
    max_backoff_ms: 60000,
  },
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

const defaultPowerActionState: PowerActionState = {
  action: "none",
  phase: "idle",
  remaining_seconds: 0,
  target_count: 0,
};

let sharedAudioContext: AudioContext | null = null;
const getAudioContext = (): AudioContext | null => {
  if (typeof window === "undefined") return null;
  const AudioContextCtor =
    window.AudioContext ??
    (window as unknown as { webkitAudioContext?: typeof AudioContext })
      .webkitAudioContext;
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

const playTone = (
  ctx: AudioContext,
  frequency: number,
  startAt: number,
  durationMs: number
) => {
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = "sine";
  osc.frequency.value = frequency;
  const durationSec = durationMs / 1000;
  gain.gain.setValueAtTime(0, startAt);
  gain.gain.linearRampToValueAtTime(0.18, startAt + 0.01);
  gain.gain.exponentialRampToValueAtTime(0.0001, startAt + durationSec);
  osc.connect(gain);
  gain.connect(ctx.destination);
  osc.start(startAt);
  osc.stop(startAt + durationSec + 0.02);
};

const playNotificationSound = async (
  kind: "completed" | "failed"
): Promise<void> => {
  const ctx = getAudioContext();
  if (!ctx) return;
  try {
    if (ctx.state === "suspended") {
      await ctx.resume();
    }
  } catch {
    return;
  }
  const tones =
    kind === "completed"
      ? [
          { freq: 523.25, dur: 120 },
          { freq: 659.25, dur: 120 },
          { freq: 783.99, dur: 160 },
        ]
      : [
          { freq: 783.99, dur: 160 },
          { freq: 659.25, dur: 160 },
          { freq: 523.25, dur: 220 },
        ];
  let t = ctx.currentTime;
  for (const tone of tones) {
    playTone(ctx, tone.freq, t, tone.dur);
    t += tone.dur / 1000;
  }
};

export default function App() {
  const appWindow = useMemo(() => (isDesktop() ? getCurrentWindow() : null), []);
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const tasksRef = useRef<DownloadTask[]>([]);
  useEffect(() => {
    tasksRef.current = tasks;
  }, [tasks]);
  const [settings, setSettings] = useState(defaults);
  const [loading, setLoading] = useState(true);
  const [fatal, setFatal] = useState<string>();
  const [filter, setFilter] = useState<FilterKey>("all");
  const [search, setSearch] = useState("");
  useLocale();

  useEffect(() => {
    if (settings.language) setLocale(settings.language);
  }, [settings.language]);

  const [historyDate, setHistoryDate] = useState(defaultHistoryDateRange);
  const [view, setView] = useState<"main" | "history">("main");
  const [historyStatusFilter, setHistoryStatusFilter] = useState<
    TaskStatus | "all"
  >("all");
  const [powerAction, setPowerAction] = useState(defaultPowerActionState);
  const [sort, setSort] = useState<{ key: keyof DownloadTask; desc: boolean }>({
    key: "queue_position",
    desc: false,
  });
  const [selected, setSelected] = useState(new Set<string>());
  const [primaryTaskId, setPrimaryTaskId] = useState<string | undefined>(
    undefined
  );
  const [showDetails, setShowDetails] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [categoriesExpanded, setCategoriesExpanded] = useState(true);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const [splash, setSplash] = useState(true);
  const [initialUrlFromClipboard, setInitialUrlFromClipboard] = useState("");
  const [toast, setToast] = useState<{ kind: "ok" | "error"; text: string }>();
  const [context, setContext] = useState<{
    x: number;
    y: number;
    id: string;
  }>();
  const [aboutOpen, setAboutOpen] = useState(false);
  const [columnWidths, setColumnWidths] = useState<Record<string, number>>({});
  const [selfcheckToast, setSelfcheckToast] = useState<
    | {
        interrupted: number;
        dropped: number;
        taskIds: string[];
      }
    | undefined
  >();

  const dragRef = useRef<{
    taskId: string;
    startY: number;
    active: boolean;
    hoverId: string | null;
    sourceEl: HTMLElement;
    dropEl: HTMLElement | null;
  } | null>(null);

  const notifyRef = useRef<(text: string, kind?: "ok" | "error") => void>(
    () => {}
  );
  const refreshRef = useRef<() => Promise<void>>(() => Promise.resolve());

  const [renameTarget, setRenameTarget] = useState<DownloadTask | null>(null);
  const [speedLimitTarget, setSpeedLimitTarget] = useState<DownloadTask | null>(
    null
  );
  const [refreshUrlTarget, setRefreshUrlTarget] = useState<DownloadTask | null>(
    null
  );
  const [failureToast, setFailureToast] = useState<
    | {
        taskId: string;
        title: string;
        body: string;
      }
    | undefined
  >();
  const [youtubeModalTaskId, setYoutubeModalTaskId] = useState<string | null>(
    null
  );

  const [tags, setTags] = useState<Tag[]>([]);
  const [taskTags, setTaskTags] = useState<TaskTagsMap>({});
  const [advancedFilter, setAdvancedFilter] = useState<AdvancedFilter>(
    EMPTY_ADVANCED_FILTER
  );
  const [advancedFilterOpen, setAdvancedFilterOpen] = useState(false);
  const [quickViews, setQuickViews] = useState<QuickView[]>([]);

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

  const handleCheckboxMouseDown = (
    taskId: string,
    isChecked: boolean,
    event: React.MouseEvent
  ) => {
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

  const taskEventSeq = useRef(0);
  const refresh = async () => {
    try {
      const seqBefore = taskEventSeq.current;
      const list = await api.list();
      setTasks(taskEventSeq.current !== seqBefore ? await api.list() : list);
      if (isDesktop()) {
        const [nextSettings, nextPowerAction] = await Promise.all([
          api.settings(),
          api.powerActionState(),
        ]);
        setSettings(nextSettings);
        setPowerAction(nextPowerAction);
      }
      setFatal(undefined);
    } catch (error) {
      setFatal(String(error));
    } finally {
      setLoading(false);
    }
  };

  const refreshTags = async () => {
    if (!isDesktop()) return;
    try {
      const [nextTags, nextTaskTags] = await Promise.all([
        api.tagList(),
        api.taskTagsListAll(),
      ]);
      setTags(nextTags);
      setTaskTags(nextTaskTags);
    } catch (error) {}
  };

  const QUICK_VIEWS_STORAGE_KEY = "maobu.quickViews";
  useEffect(() => {
    if (!isDesktop()) {
      try {
        const raw = localStorage.getItem(QUICK_VIEWS_STORAGE_KEY);
        if (raw) {
          const parsed = JSON.parse(raw) as QuickView[];
          if (Array.isArray(parsed)) setQuickViews(parsed);
        }
      } catch {}
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        let views = await api.savedViewList();
        if (views.length === 0) {
          try {
            const raw = localStorage.getItem(QUICK_VIEWS_STORAGE_KEY);
            if (raw) {
              const parsed = JSON.parse(raw) as QuickView[];
              const valid = Array.isArray(parsed)
                ? parsed.filter(
                    (v) =>
                      v &&
                      typeof v.id === "string" &&
                      v.name &&
                      typeof v.name === "string" &&
                      v.filter &&
                      typeof v.filter === "object"
                  )
                : [];
              if (valid.length > 0) {
                await api.savedViewReplaceAll(valid);
                views = valid;
              }
            }
            localStorage.removeItem(QUICK_VIEWS_STORAGE_KEY);
          } catch {}
        }
        if (!cancelled) setQuickViews(views);
      } catch {}
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const reload = () => {
      void api.savedViewList().then(setQuickViews).catch(() => {});
    };
    window.addEventListener("maobu:backup-restored", reload);
    return () => window.removeEventListener("maobu:backup-restored", reload);
  }, []);

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
            if (isDesktop() && appWindow) {
              void appWindow.show();
              void appWindow.unminimize();
              void appWindow.setFocus();
            }
          }, 300);
        } else {
          setSplash(false);
          if (isDesktop() && appWindow) {
            void appWindow.show();
            void appWindow.unminimize();
            void appWindow.setFocus();
          }
        }
      }, delay);
    });

    let unlisten: Array<() => void> = [];
    let disposed = false;
    const keep = (item: (() => void) | null | undefined) => {
      if (!item) return;
      if (disposed) item();
      else unlisten.push(item);
    };
    void api
      .subscribe((event) => {
        taskEventSeq.current += 1;
        if ("removed" in event) {
          setTasks((items) =>
            items.filter((task) => task.id !== event.removed)
          );
          setSelected((current) => {
            if (current.has(event.removed)) {
              const next = new Set(current);
              next.delete(event.removed);
              return next;
            }
            return current;
          });
        } else {
          setTasks((items) =>
            items.some((task) => task.id === event.task.id)
              ? items.map((task) =>
                  task.id === event.task.id ? event.task : task
                )
              : [event.task, ...items]
          );
        }
      })
      .then((items) => {
        if (disposed) items.forEach((fn) => fn());
        else unlisten.push(...items);
      });
    void api.subscribeSettings(setSettings).then((item) => {
      keep(item);
    });
    void api.subscribePowerAction(setPowerAction).then((item) => {
      keep(item);
    });
    void api
      .subscribeNotificationErrors((message) =>
        setToast({ kind: "error", text: message })
      )
      .then((item) => {
        keep(item);
      });
    void api
      .subscribeStartupSelfcheck((report: SelfcheckReport) => {
        if (report.interrupted_count > 0 || report.dropped_shards > 0) {
          setSelfcheckToast({
            interrupted: report.interrupted_count,
            dropped: report.dropped_shards,
            taskIds: report.recovered_tasks ?? [],
          });
        }
      })
      .then((item) => {
        keep(item);
      });
    void api
      .subscribeDeepLinkErrors((message) => {
        setToast({ kind: "error", text: message });
      })
      .then((item) => {
        keep(item);
      });
    void api
      .subscribeDeepLinkReceived((payload: DeepLinkReceivedPayload) => {
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
      })
      .then((item) => {
        keep(item);
      });
    void api
      .subscribeMeteredNetwork((payload: MeteredNetworkDetectedPayload) => {
        const count = payload.paused_count ?? 0;
        if (count > 0) {
          setToast({
            kind: "error",
            text: `当前为计量网络，已暂停 ${count} 个任务`,
          });
        }
      })
      .then((item) => {
        keep(item);
      });
    if (isDesktop()) {
      void onAction((notification) => {
        if (appWindow) {
          void appWindow.show();
          void appWindow.unminimize();
          void appWindow.setFocus();
        }
        const taskId = (
          notification as { extra?: { task_id?: string } }
        )?.extra?.task_id;
        if (taskId) {
          setSelected(new Set([taskId]));
          requestShowDetails(true);
          setView("main");
          setFilter("all");
        }
      }).then((item) => {
        if (item && !disposed)
          unlisten.push(() => {
            void item.unregister();
          });
        else if (item) void item.unregister();
      });
      void listen<string>("notification-focus-task", (event) => {
        setSelected(new Set([event.payload]));
        requestShowDetails(true);
        setView("main");
        setFilter("all");
      }).then((item) => {
        keep(item);
      });
    }
    return () => {
      disposed = true;
      document.removeEventListener("contextmenu", handleContextMenu);
      unlisten.forEach((item) => item());
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void api
      .subscribeTaskNotification((payload: TaskNotificationPayload) => {
        if (payload.kind === "completed") {
          if (settings.notify_sound_enabled) {
            void playNotificationSound("completed");
          }
        } else if (payload.kind === "failed") {
          if (settings.notify_failure_sound_enabled) {
            void playNotificationSound("failed");
          }
          setFailureToast({
            taskId: payload.task_id,
            title: payload.title,
            body: payload.body,
          });
        }
      })
      .then((item) => {
        if (item) unlisten = item;
      });
    return () => {
      if (unlisten) unlisten();
    };
  }, [settings.notify_sound_enabled, settings.notify_failure_sound_enabled]);

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
      document.body.classList.toggle("dark", dark);
      document.body.classList.toggle("light", !dark);
      void applyWindowAppearance(settings.frosted_glass, dark).catch(
        (error) => {
          document.documentElement.dataset.windowStyle = "solid";
          setToast({
            kind: "error",
            text: `无法应用磨砂玻璃效果：${String(error)}`,
          });
        }
      );
    };
    applyColorScheme();
    if (settings.color_scheme !== "system") return;
    const media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", applyColorScheme);
    return () => media.removeEventListener("change", applyColorScheme);
  }, [settings.color_scheme, settings.accent_color, settings.frosted_glass]);

  useEffect(() => {
    document.body.classList.toggle("row-compact", settings.row_compact);
  }, [settings.row_compact]);

  useEffect(() => {
    if (!appWindow || !settings.window_width || !settings.window_height) return;
    void appWindow.setSize(
      new LogicalSize(settings.window_width, settings.window_height)
    );
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

  const handleCloseConfirm = async (
    action: "tray" | "exit",
    remember: boolean
  ) => {
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
          const firstLine = text.trim().split(/\r?\n/)[0] || "";
          const magnet =
            firstLine.toLowerCase().startsWith("magnet:") &&
            settings.bt_intercept_magnet !== false
              ? firstLine
              : null;
          const match = text.match(/https?:\/\/[^\s<>"']+/i);
          const picked =
            match && isDownloadableUrl(match[0]) ? match[0] : magnet;
          if (picked) {
            setInitialUrlFromClipboard(picked);
            setNewOpen(true);
            if (appWindow) {
              await appWindow.show();
              await appWindow.unminimize();
              await appWindow.setFocus();
            }
          }
        }
      } catch (e) {}
    }, 1500);
    return () => clearInterval(interval);
  }, [settings.clipboard_monitor]);

  const [dragOverlay, setDragOverlay] = useState(false);
  const dragDepth = useRef(0);
  const [droppedTorrent, setDroppedTorrent] = useState<{
    name: string;
    base64: string;
  } | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    const isRelevantDrag = (event: DragEvent) => {
      const types = Array.from(event.dataTransfer?.types ?? []);
      return (
        types.includes("Files") ||
        types.includes("text/uri-list") ||
        types.includes("text/plain")
      );
    };
    const onDragEnter = (event: DragEvent) => {
      if (!isRelevantDrag(event)) return;
      event.preventDefault();
      dragDepth.current += 1;
      setDragOverlay(true);
    };
    const onDragOver = (event: DragEvent) => {
      if (!isRelevantDrag(event)) return;
      event.preventDefault();
    };
    const onDragLeave = (event: DragEvent) => {
      if (!isRelevantDrag(event)) return;
      dragDepth.current = Math.max(0, dragDepth.current - 1);
      if (dragDepth.current === 0) setDragOverlay(false);
    };
    const onDrop = (event: DragEvent) => {
      event.preventDefault();
      dragDepth.current = 0;
      setDragOverlay(false);
      if (
        newOpen ||
        settingsOpen ||
        renameTarget ||
        speedLimitTarget ||
        aboutOpen ||
        showCloseConfirm ||
        context
      )
        return;
      const transfer = event.dataTransfer;
      if (!transfer) return;
      const files = Array.from(transfer.files ?? []);
      if (files.length > 0) {
        const { torrents, rejected } = classifyDroppedFiles(files);
        if (torrents.length === 0) {
          notifyRef.current(t("toasts.dropOnlyTorrent"), "error");
          return;
        }
        const first = torrents[0];
        if (first.size > MAX_DROPPED_TORRENT_BYTES) {
          notifyRef.current(
            t("toasts.dropTorrentTooLarge", {
              size: formatBytes(first.size),
            }),
            "error"
          );
          return;
        }
        void (async () => {
          try {
            const base64 = arrayBufferToBase64(await first.arrayBuffer());
            setDroppedTorrent({ name: first.name, base64 });
            setInitialUrlFromClipboard("");
            setNewOpen(true);
            if (torrents.length > 1) {
              notifyRef.current(
                t("toasts.dropTorrentMultiple", { count: torrents.length })
              );
            }
            if (rejected.length > 0) {
              notifyRef.current(
                t("toasts.dropIgnoredFiles", { count: rejected.length })
              );
            }
          } catch (error) {
            notifyRef.current(
              `${t("toasts.dropTorrentReadFailed")}：${String(error)}`,
              "error"
            );
          }
        })();
        return;
      }
      const text =
        transfer.getData("text/uri-list") || transfer.getData("text/plain");
      const urls = extractDroppedUrls(text);
      if (urls.length === 0) {
        notifyRef.current(t("toasts.dropNoUrl"), "error");
        return;
      }
      setDroppedTorrent(null);
      setInitialUrlFromClipboard(urls.join("\n"));
      setNewOpen(true);
    };
    window.addEventListener("dragenter", onDragEnter);
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("dragleave", onDragLeave);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("dragenter", onDragEnter);
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("dragleave", onDragLeave);
      window.removeEventListener("drop", onDrop);
    };
  }, [
    newOpen,
    settingsOpen,
    renameTarget,
    speedLimitTarget,
    aboutOpen,
    showCloseConfirm,
    context,
  ]);

  const lastSelectedTaskId = useRef<string | undefined>(undefined);
  const lastSelectedCount = useRef(0);
  const skipAutoCollapseRef = useRef(false);
  const requestShowDetails = (value: boolean) => {
    skipAutoCollapseRef.current = true;
    setShowDetails(value);
  };

  const partitioned = useMemo(() => {
    const archiveMs = Math.max(0, settings.archive_days) * 86_400_000;
    const threshold = Math.max(0, settings.archive_threshold);
    const now = Date.now();
    const isOldCompleted = (task: DownloadTask) =>
      task.status === "completed" &&
      task.completed_at != null &&
      archiveMs > 0 &&
      now - task.completed_at > archiveMs;
    const mainCompletedSorted = tasks
      .filter((task) => task.status === "completed" && !isOldCompleted(task))
      .sort((a, b) => (a.completed_at ?? 0) - (b.completed_at ?? 0));
    const overflowCount = Math.max(0, mainCompletedSorted.length - threshold);
    const overflowIds = new Set(
      mainCompletedSorted.slice(0, overflowCount).map((task) => task.id)
    );
    const mainTasks: DownloadTask[] = [];
    const historyTasks: DownloadTask[] = [];
    for (const task of tasks) {
      if (
        task.status === "cancelled" ||
        isOldCompleted(task) ||
        overflowIds.has(task.id)
      ) {
        historyTasks.push(task);
      } else {
        mainTasks.push(task);
      }
    }
    return { mainTasks, historyTasks };
  }, [tasks, settings.archive_days, settings.archive_threshold]);

  const visible = useMemo(() => {
    const source =
      view === "history" ? partitioned.historyTasks : partitioned.mainTasks;
    return source
      .filter((task) => {
        if (view === "history") {
          const statusOk =
            historyStatusFilter === "all" ||
            task.status === historyStatusFilter;
          const date = matchesHistoryDate(task.completed_at, historyDate);
          return (
            statusOk &&
            date &&
            `${task.file_name} ${task.url}`
              .toLowerCase()
              .includes(search.toLowerCase())
          );
        }
        const category = getCategories().some(([key]) => key === filter)
          ? filter === "bt"
            ? task.task_kind === "bt" || task.category === "bt"
            : task.category === filter
          : true;
        const status = getNav().some(
          ([key]) => key === filter && key !== "all"
        )
          ? task.status === filter
          : true;
        const date =
          filter !== "completed" ||
          matchesHistoryDate(task.completed_at, historyDate);
        const searchOk = `${task.file_name} ${task.url}`
          .toLowerCase()
          .includes(search.toLowerCase());
        const advancedOk = matchesAdvancedFilter(
          task,
          advancedFilter,
          taskTags[task.id] ?? []
        );
        return category && status && date && searchOk && advancedOk;
      })
      .sort((a, b) => {
        const av = a[sort.key] ?? "";
        const bv = b[sort.key] ?? "";
        const result =
          typeof av === "number" && typeof bv === "number"
            ? av - bv
            : String(av).localeCompare(String(bv));
        return sort.desc ? -result : result;
      });
  }, [
    partitioned,
    view,
    filter,
    historyDate,
    search,
    sort,
    historyStatusFilter,
    advancedFilter,
    taskTags,
  ]);

  const selectedTasks = tasks.filter((task) => selected.has(task.id));
  const selectedOne =
    selectedTasks.length === 1 ? selectedTasks[0] : undefined;
  const activeTask = useMemo(() => {
    if (selected.size === 0) return undefined;
    if (primaryTaskId && selected.has(primaryTaskId)) {
      const found = tasks.find((t) => t.id === primaryTaskId);
      if (found) return found;
    }
    return selectedTasks.length > 0
      ? selectedTasks[selectedTasks.length - 1]
      : undefined;
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
        skipAutoCollapseRef.current = false;
      } else {
        setShowDetails(!settings.detail_default_collapsed);
      }
    }
    lastSelectedCount.current = currentCount;
    lastSelectedTaskId.current = currentTaskId;
  }, [selected, activeTask, settings.detail_default_collapsed]);

  const active = tasks.filter((task) => task.status === "downloading");
  const totalSpeed = active.reduce((sum, task) => sum + task.speed, 0);
  const notify = (text: string, kind: "ok" | "error" = "ok") =>
    setToast({ text, kind });
  notifyRef.current = notify;
  refreshRef.current = refresh;

  const armPowerAction = async (action: Exclude<PowerAction, "none">) => {
    try {
      setPowerAction(await api.armPowerAction(action));
      notify(
        action === "shutdown"
          ? "已设置队列完成后关机"
          : "已设置队列完成后休眠"
      );
    } catch (error) {
      notify(String(error), "error");
      throw error;
    }
  };

  const cancelPowerAction = async () => {
    try {
      setPowerAction(await api.cancelPowerAction());
      notify("已取消队列完成后的系统操作");
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const bulk = async (action: string) => {
    try {
      if (
        view === "history" &&
        (action === "resume" || action === "redownload")
      ) {
        for (const id of selected) {
          await api.action(id, "redownload");
        }
        setSelected(new Set());
        notify("已重新加入下载队列并开始下载");
        return;
      }
      const ids =
        action === "resume"
          ? [...selected].filter((id) => {
              const t = tasks.find((task) => task.id === id);
              return t && !["completed", "cancelled"].includes(t.status);
            })
          : [...selected];
      if (ids.length === 0) return;
      await api.bulkAction(ids, action);
      notify(action === "pause" ? "已暂停所选任务" : "任务已加入队列");
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeSelected = async (deleteFile: boolean) => {
    try {
      const isHistory = view === "history";
      const selectedList = tasks.filter((t) => selected.has(t.id));
      const hasIncomplete = selectedList.some(
        (t) => t.status !== "completed"
      );
      for (const id of selected) {
        if (isHistory) {
          await api.remove(id, deleteFile);
        } else {
          await api.archive(id, deleteFile);
        }
      }
      setSelected(new Set());
      notify(
        isHistory
          ? deleteFile
            ? "已从历史中彻底删除任务及文件"
            : "已从历史中彻底删除任务记录"
          : deleteFile
          ? "任务文件已删除，下载链接已归档至历史记录"
          : hasIncomplete
          ? "未完成任务已清理，下载链接已保留至历史记录"
          : "任务已移入历史记录（可在历史中随时重新下载）"
      );
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const clearHistory = async (deleteFile: boolean) => {
    try {
      for (const task of partitioned.historyTasks)
        await api.remove(task.id, deleteFile);
      setSelected(new Set());
      notify(
        deleteFile
          ? `已删除 ${partitioned.historyTasks.length} 个历史任务及文件`
          : `已删除 ${partitioned.historyTasks.length} 个历史任务记录`
      );
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const handleTaskMouseDown = useCallback(
    (task: DownloadTask, event: React.MouseEvent) => {
      if (event.button !== 0) return;
      const target = event.target as HTMLElement;
      if (target.closest("input, button, label")) return;
      event.preventDefault();
      dragRef.current = {
        taskId: task.id,
        startY: event.clientY,
        active: false,
        hoverId: null,
        sourceEl: event.currentTarget as HTMLElement,
        dropEl: null,
      };
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
        const hoverId =
          rowEl && rowEl.dataset.taskId !== ref.taskId
            ? rowEl.dataset.taskId ?? null
            : null;
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
        if (ref?.dropEl) {
          ref.dropEl.classList.remove("drop-target");
        }
        if (!ref?.active) return;
        const targetId = ref.hoverId;
        if (!targetId || targetId === ref.taskId) return;
        const currentTasks = tasksRef.current;
        const dragged = currentTasks.find((t) => t.id === ref.taskId);
        const dropTarget = currentTasks.find((t) => t.id === targetId);
        if (!dragged || !dropTarget) return;
        if (dragged.priority !== dropTarget.priority) {
          notifyRef.current(
            "请通过右键菜单或数字优先级调整跨优先级排序",
            "error"
          );
          return;
        }
        const reorderedIds = reorderTaskIdsWithinPriority(
          currentTasks,
          dragged.id,
          dropTarget.id
        );
        if (!reorderedIds) return;
        try {
          await api.reorder(reorderedIds);
          const positions = new Map(
            reorderedIds.map((id, index) => [id, index])
          );
          setTasks((items) =>
            items.map((item) => {
              const position = positions.get(item.id);
              return position === undefined
                ? item
                : { ...item, queue_position: position };
            })
          );
          setSort({ key: "queue_position", desc: false });
          void refreshRef.current();
          notifyRef.current("队列顺序已更新");
        } catch (error) {
          notifyRef.current(String(error), "error");
        }
      };
      window.addEventListener("mousemove", handleMouseMove, true);
      window.addEventListener("mouseup", handleMouseUp, true);
    },
    []
  );

  const beginResize = (key: string, event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const start = event.clientX;
    const defaultWidths: Record<string, number> = {
      size: 78,
      status: 82,
      connection: 64,
      progress: 130,
      speed: 78,
      eta: 82,
      created: 100,
    };
    const isSmallScreen = window.innerWidth <= 1180;
    const fallbackWidth = isSmallScreen
      ? ({
          size: 70,
          status: 82,
          connection: 58,
          progress: 112,
          speed: 72,
          eta: 76,
          created: 92,
        }[key] ?? 70)
      : defaultWidths[key] ?? 80;
    const width = columnWidths[key] ?? fallbackWidth;
    const move = (next: globalThis.MouseEvent) =>
      setColumnWidths((value) => ({
        ...value,
        [key]: Math.max(58, width + next.clientX - start),
      }));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const handleDeleteTasks = useCallback(
    async (taskIds: Set<string>, deleteFile: boolean) => {
      if (taskIds.size === 0) return;
      const isHistoryView = view === "history";
      const taskList = tasks.filter((t) => taskIds.has(t.id));
      const hasIncomplete = taskList.some((t) => t.status !== "completed");
      const succeeded: string[] = [];
      try {
        for (const id of taskIds) {
          if (isHistoryView) {
            await api.remove(id, deleteFile);
          } else {
            await api.archive(id, deleteFile);
          }
          succeeded.push(id);
        }
        const count = succeeded.length;
        notify(
          isHistoryView
            ? deleteFile
              ? count === 1
                ? "已从历史中彻底删除任务及文件"
                : `已从历史中彻底删除 ${count} 个任务及文件`
              : count === 1
              ? "已从历史中彻底删除任务记录"
              : `已从历史中彻底删除 ${count} 个任务记录`
            : deleteFile
            ? count === 1
              ? "任务文件已删除，下载链接已归档至历史记录"
              : `${count} 个任务文件已删除，链接已归档至历史`
            : hasIncomplete
            ? count === 1
              ? "任务及未完成文件已清理，下载链接已保留至历史记录"
              : `${count} 个任务及未完成文件已清理，链接已保留至历史`
            : count === 1
            ? "任务已移入历史记录（可随时在历史中重新下载）"
            : `${count} 个任务已移入历史记录`
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
          setPrimaryTaskId((prev) =>
            prev && succeeded.includes(prev) ? undefined : prev
          );
        }
      }
    },
    [notify, tasks, view]
  );

  useEffect(() => {
    const isEditing = () => {
      const active = document.activeElement as HTMLElement | null;
      if (!active) return false;
      const tag = active.tagName;
      return tag === "INPUT" || tag === "TEXTAREA" || active.isContentEditable;
    };
    const handler = (event: KeyboardEvent) => {
      if (
        newOpen ||
        settingsOpen ||
        renameTarget ||
        speedLimitTarget ||
        aboutOpen ||
        showCloseConfirm ||
        context
      )
        return;
      if (isEditing()) return;

      const keys = settings.shortcut_keys || DEFAULT_SHORTCUTS;

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        if (visible.length === 0) return;
        event.preventDefault();
        const delta = event.key === "ArrowDown" ? 1 : -1;
        const currentId = selected.size === 1 ? [...selected][0] : null;
        const currentIndex = currentId
          ? visible.findIndex((task) => task.id === currentId)
          : -1;
        const nextIndex =
          currentIndex === -1
            ? delta === 1
              ? 0
              : visible.length - 1
            : Math.min(visible.length - 1, Math.max(0, currentIndex + delta));
        const nextTask = visible[nextIndex];
        setSelected(new Set([nextTask.id]));
        setPrimaryTaskId(nextTask.id);
        requestAnimationFrame(() => {
          document
            .querySelector(`.task-row[data-task-id="${nextTask.id}"]`)
            ?.scrollIntoView({ block: "nearest" });
        });
        return;
      }

      if (matchesShortcut(event, keys.new_task)) {
        event.preventDefault();
        setNewOpen(true);
        return;
      }
      if (matchesShortcut(event, keys.select_all)) {
        if (visible.length === 0) return;
        event.preventDefault();
        const allSelected =
          visible.length > 0 &&
          selected.size === visible.length &&
          visible.every((t) => selected.has(t.id));
        if (allSelected) {
          setSelected(new Set());
        } else {
          setSelected(new Set(visible.map((task) => task.id)));
        }
        return;
      }
      if (matchesShortcut(event, keys.copy_url)) {
        if (selectedTasks.length === 0) return;
        event.preventDefault();
        const text = selectedTasks.map((task) => task.url).join("\n");
        void navigator.clipboard
          .writeText(text)
          .then(() => notify(`已复制 ${selectedTasks.length} 个来源 URL`))
          .catch((error) =>
            notify(`复制 URL 失败：${String(error)}`, "error")
          );
        return;
      }
      if (matchesShortcut(event, keys.open_folder)) {
        if (!selectedOne || selectedOne.status !== "completed") return;
        event.preventDefault();
        void api
          .openFolder(selectedOne.id)
          .catch((error) => notify(String(error), "error"));
        return;
      }
      if (matchesShortcut(event, keys.delete_file)) {
        if (selected.size === 0) return;
        event.preventDefault();
        void handleDeleteTasks(new Set(selected), true);
        return;
      }
      if (matchesShortcut(event, keys.delete_task)) {
        if (selected.size === 0) return;
        event.preventDefault();
        void handleDeleteTasks(new Set(selected), false);
        return;
      }
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
      if (matchesShortcut(event, keys.toggle_pause) && !event.repeat) {
        if (selected.size === 0) return;
        event.preventDefault();
        const anyActive = tasks.some(
          (task) =>
            selected.has(task.id) &&
            [
              "downloading",
              "waiting-network",
              "connecting",
              "verifying",
              "extracting",
            ].includes(task.status)
        );
        void bulk(anyActive ? "pause" : "resume");
        return;
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [
    selected,
    tasks,
    visible,
    selectedTasks,
    selectedOne,
    newOpen,
    settingsOpen,
    renameTarget,
    speedLimitTarget,
    aboutOpen,
    showCloseConfirm,
    context,
    view,
    handleDeleteTasks,
    settings.shortcut_keys,
    notify,
    bulk,
  ]);

  const titlebar = isDesktop() ? <Titlebar /> : null;

  const globalToastLayer = (
    <>
      {toast && (
        <div className="toast">
          <span>
            {toast.kind === "ok" ? (
              <Check size={14} />
            ) : (
              <AlertCircle size={14} />
            )}
          </span>
          {toast.text}
        </div>
      )}
      {selfcheckToast && (
        <div className="toast toast-with-action" role="status">
          <span className="toast-icon">
            <AlertTriangle size={14} />
          </span>
          <div className="toast-body">
            <span>
              {t("toasts.recoveredInterrupted", {
                count: selfcheckToast.interrupted,
                dropped:
                  selfcheckToast.dropped > 0
                    ? t("toasts.recoveredDroppedShards", {
                        count: selfcheckToast.dropped,
                      })
                    : "",
              })}
            </span>
            <button
              className="toast-action-btn"
              onClick={() => {
                const ids = selfcheckToast.taskIds;
                setSelfcheckToast(undefined);
                setView("main");
                if (ids.length > 0) {
                  setSelected(new Set(ids));
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
          <button
            className="toast-close-btn"
            onClick={() => setSelfcheckToast(undefined)}
            aria-label={t("common.close")}
          >
            <X size={11} />
          </button>
        </div>
      )}
      {failureToast && (
        <div className="toast toast-with-action" role="alert">
          <span className="toast-icon">
            <AlertCircle size={14} />
          </span>
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
                    setView("main");
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
                  setView("main");
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
          <button
            className="toast-close-btn"
            onClick={() => setFailureToast(undefined)}
            aria-label={t("common.close")}
          >
            <X size={11} />
          </button>
        </div>
      )}
    </>
  );

  if (settingsOpen)
    return (
      <div className="app-container">
        {titlebar}
        <SettingsPage
          value={settings}
          onChange={setSettings}
          onClose={() => setSettingsOpen(false)}
          notify={notify}
          totalSpeed={totalSpeed}
          activeCount={active.length}
        />
        <WindowResizeHandles />
        {globalToastLayer}
        {showCloseConfirm && (
          <CloseConfirmDialog
            onClose={() => setShowCloseConfirm(false)}
            onConfirm={handleCloseConfirm}
          />
        )}
      </div>
    );

  const sectionTitle =
    view === "history"
      ? t("nav.history")
      : ([...getNav(), ...getCategories()].find(
          ([key]) => key === filter
        )?.[1] ?? t("nav.allTasks"));
  const showCompletedAt = view === "history" || filter === "completed";

  return (
    <div className="app-container">
      {titlebar}
      <div className="app-frame">
        <aside className="nav-pane">
          <div
            className="brand"
            onClick={() => setAboutOpen(true)}
            title={t("app.about")}
          >
            <div className="app-icon">
              <CatDownloadMark />
            </div>
            <span>
              <b>{t("app.name")}</b>
              <small>{t("app.nameEn")}</small>
            </span>
          </div>
          <button className="new-button" onClick={() => setNewOpen(true)}>
            <Plus size={15} />
            {t("nav.newTask")}
          </button>
          <div className="nav-scroll">
            <p className="nav-label">{t("nav.tasks")}</p>
            {getNav().map(([key, label, Icon]) => (
              <button
                key={key}
                className={
                  filter === key && view === "main"
                    ? "nav-item active"
                    : "nav-item"
                }
                onClick={() => {
                  setView("main");
                  setFilter(key);
                  setSelected(new Set());
                  setPrimaryTaskId(undefined);
                }}
              >
                <Icon size={14} />
                <span>{label}</span>
                <small>
                  {key === "all"
                    ? partitioned.mainTasks.length
                    : partitioned.mainTasks.filter(
                        (task) => task.status === key
                      ).length}
                </small>
              </button>
            ))}
            <p
              className="nav-label interactive"
              onClick={() => setCategoriesExpanded(!categoriesExpanded)}
            >
              <span>{t("nav.types")}</span>
              <span
                className={`nav-label-chevron ${
                  categoriesExpanded ? "" : "collapsed"
                }`}
              >
                <ChevronDown size={12} />
              </span>
            </p>
            {categoriesExpanded && (
              <div className="nav-grid">
                {getCategories().map(([key, label, Icon]) => (
                  <button
                    key={key}
                    className={
                      filter === key && view === "main"
                        ? "nav-item active"
                        : "nav-item"
                    }
                    onClick={() => {
                      setView("main");
                      setFilter(key);
                      setSelected(new Set());
                      setPrimaryTaskId(undefined);
                    }}
                  >
                    <Icon size={14} />
                    <span>{label}</span>
                    <small>
                      {partitioned.mainTasks.filter((task) =>
                        key === "bt"
                          ? task.task_kind === "bt" || task.category === "bt"
                          : task.category === key
                      ).length || ""}
                    </small>
                  </button>
                ))}
              </div>
            )}
            <p className="nav-label">{t("nav.archive")}</p>
            <button
              className={view === "history" ? "nav-item active" : "nav-item"}
              onClick={() => {
                setView("history");
                setSelected(new Set());
                setPrimaryTaskId(undefined);
              }}
              title={t("nav.historyArchive")}
            >
              <Archive size={14} />
              <span>{t("nav.history")}</span>
              <small>{partitioned.historyTasks.length || ""}</small>
            </button>
          </div>
          <div className="nav-footer">
            <button
              className="nav-settings"
              onClick={() => setSettingsOpen(true)}
            >
              <Settings size={15} />
              <span>{t("nav.settings")}</span>
            </button>
            <div
              className="nav-status"
              onClick={() => setSettingsOpen(true)}
            >
              <i
                className={
                  isDesktop() ? "status-dot online" : "status-dot offline"
                }
              />
              <span>
                {t("nav.speedFormat", {
                  speed: `${formatBytes(totalSpeed)}/s`,
                  count: active.length,
                })}
              </span>
            </div>
          </div>
        </aside>
        <main className="workspace">
          <header className="titlebar" data-tauri-drag-region>
            <h1 data-tauri-drag-region>{sectionTitle}</h1>
            <label className="search-box">
              <Search size={14} />
              <input
                aria-label={t("toolbar.searchAria")}
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("toolbar.searchPlaceholder")}
              />
              {search && (
                <button onClick={() => setSearch("")}>
                  <X size={13} />
                </button>
              )}
            </label>
            <div className="toolbar-actions">
              <button
                className="action-btn-standalone"
                onClick={() => setNewOpen(true)}
                title={t("toolbar.newTask")}
              >
                <Plus size={14} />
              </button>

              {view === "main" && (
                <button
                  className={`action-btn-standalone${
                    advancedFilterOpen ? " active" : ""
                  }`}
                  onClick={() => setAdvancedFilterOpen((v) => !v)}
                  title={t("toolbar.advancedFilter")}
                  aria-pressed={advancedFilterOpen}
                >
                  <ListFilter size={14} />
                  {!isAdvancedFilterEmpty(advancedFilter) && (
                    <span
                      className="filter-badge"
                      aria-label={t("toolbar.filterApplied")}
                    />
                  )}
                </button>
              )}

              <div className="action-group">
                <button
                  disabled={!selected.size}
                  onClick={() => void bulk("resume")}
                  title={t("toolbar.startTask")}
                >
                  <Play size={14} />
                </button>
                <button
                  disabled={!selected.size}
                  onClick={() => void bulk("pause")}
                  title={t("toolbar.pauseTask")}
                >
                  <Pause size={14} />
                </button>
                <button
                  className="danger-action"
                  disabled={!selected.size}
                  onClick={() => void removeSelected(false)}
                  title={t("toolbar.deleteRecord")}
                >
                  <Trash2 size={14} />
                </button>
              </div>

              <div className="action-group">
                <button
                  disabled={!selectedOne || selectedOne.status !== "completed"}
                  onClick={() =>
                    selectedOne &&
                    void api
                      .openFile(selectedOne.id)
                      .catch((error) => notify(String(error), "error"))
                  }
                  title={t("toolbar.openFile")}
                >
                  <ExternalLink size={14} />
                </button>
                <button
                  disabled={!selectedOne}
                  onClick={() =>
                    selectedOne &&
                    void api
                      .openFolder(selectedOne.id)
                      .catch((error) => notify(String(error), "error"))
                  }
                  title={t("toolbar.openFolder")}
                >
                  <FolderOpen size={14} />
                </button>
              </div>

              {view === "history" && (
                <button
                  className="action-btn-standalone danger-action"
                  disabled={partitioned.historyTasks.length === 0}
                  onClick={() => void clearHistory(true)}
                  title={t("toolbar.clearHistory")}
                >
                  <Trash2 size={14} />
                </button>
              )}

              <button
                className="action-btn-standalone"
                onClick={() => void refresh()}
                title={t("toolbar.refreshList")}
              >
                <RefreshCw size={14} />
              </button>
              <PowerActionButton
                state={powerAction}
                onArm={armPowerAction}
                onCancel={cancelPowerAction}
              />
            </div>
            <button
              className="details-toggle"
              onClick={() => setShowDetails((value) => !value)}
              title={t("toolbar.detailsPanel")}
            >
              {showDetails ? (
                <PanelRightClose size={15} />
              ) : (
                <PanelRightOpen size={15} />
              )}
            </button>
          </header>
          {fatal && (
            <div className="error-banner">
              <Unplug size={16} />
              <span>{t("toasts.kernelConnectionFailed", { error: fatal })}</span>
              <button onClick={() => void refresh()}>{t("common.retry")}</button>
            </div>
          )}
          <PowerActionBanner state={powerAction} onCancel={cancelPowerAction} />
          {view === "history" ? (
            <div
              className="history-filter-bar"
              aria-label={t("historyFilter.status")}
            >
              <span>{t("historyFilter.status")}</span>
              <Select
                value={historyStatusFilter}
                onChange={(val: any) =>
                  setHistoryStatusFilter(val as TaskStatus | "all")
                }
                options={[
                  { value: "all", label: t("historyFilter.allStatuses") },
                  { value: "completed", label: t("status.completed") },
                  { value: "failed", label: t("status.failed") },
                  { value: "cancelled", label: t("status.cancelled") },
                  { value: "interrupted", label: t("status.interrupted") },
                ]}
                ariaLabel={t("historyFilter.status")}
              />
              <span className="history-filter-separator" aria-hidden="true">
                ·
              </span>
              <span>{t("historyFilter.completionDate")}</span>
              <Select
                value={historyDate.preset}
                onChange={(val: any) =>
                  setHistoryDate({
                    ...historyDate,
                    preset: val as typeof historyDate.preset,
                  })
                }
                options={[
                  { value: "all", label: t("historyFilter.allTime") },
                  { value: "today", label: t("historyFilter.today") },
                  { value: "7-days", label: t("historyFilter.last7Days") },
                  { value: "30-days", label: t("historyFilter.last30Days") },
                  { value: "custom", label: t("historyFilter.customRange") },
                ]}
                ariaLabel={t("historyFilter.completionDate")}
              />
              {historyDate.preset === "custom" && (
                <>
                  <input
                    type="date"
                    aria-label={t("historyFilter.startDate")}
                    value={historyDate.start}
                    onChange={(event) =>
                      setHistoryDate({
                        ...historyDate,
                        start: event.target.value,
                      })
                    }
                  />
                  <span>{t("historyFilter.to")}</span>
                  <input
                    type="date"
                    aria-label={t("historyFilter.endDate")}
                    value={historyDate.end}
                    min={historyDate.start || undefined}
                    onChange={(event) =>
                      setHistoryDate({
                        ...historyDate,
                        end: event.target.value,
                      })
                    }
                  />
                </>
              )}
            </div>
          ) : (
            filter === "completed" && (
              <HistoryDateFilter
                value={historyDate}
                onChange={setHistoryDate}
              />
            )
          )}
          {view === "main" && advancedFilterOpen && (
            <AdvancedFilterPanel
              value={advancedFilter}
              onChange={setAdvancedFilter}
              tags={tags}
              quickViews={quickViews}
              onApplyQuickView={(qv) =>
                setAdvancedFilter({ ...qv.filter })
              }
              onSaveQuickView={(name) => {
                const view: QuickView = {
                  id: newQuickViewId(),
                  name,
                  filter: advancedFilter,
                };
                setQuickViews((current) => [...current, view]);
                void api.savedViewUpsert(view).catch(() => {});
              }}
              onDeleteQuickView={(id) => {
                setQuickViews((current) =>
                  current.filter((qv) => qv.id !== id)
                );
                void api.savedViewDelete(id).catch(() => {});
              }}
              onClear={() => setAdvancedFilter({ ...EMPTY_ADVANCED_FILTER })}
            />
          )}
          <section
            className={
              showDetails ? "content-grid details-on" : "content-grid"
            }
          >
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
                <div className="table-header">
                  <label>
                    <input
                      type="checkbox"
                      aria-label={t("toolbar.selectAll")}
                      checked={
                        visible.length > 0 &&
                        visible.every((task) => selected.has(task.id))
                      }
                      onChange={() =>
                        setSelected(
                          visible.every((task) => selected.has(task.id))
                            ? new Set()
                            : new Set(visible.map((task) => task.id))
                        )
                      }
                    />
                  </label>
                  {[
                    ["file_name", t("table.fileName"), ""],
                    ["total_bytes", t("table.size"), "size"],
                    ["status", t("table.status"), "status"],
                    ["connection_count", t("table.connection"), "connection"],
                    ["downloaded_bytes", t("table.progress"), "progress"],
                    ["speed", t("table.speed"), "speed"],
                    ["eta_seconds", t("table.eta"), "eta"],
                    [
                      showCompletedAt ? "completed_at" : "created_at",
                      showCompletedAt
                        ? t("table.completedAt")
                        : t("table.createdAt"),
                      "created",
                    ],
                  ].map(([key, label, widthKey]) => (
                    <span
                      key={key}
                      onClick={() =>
                        setSort((current) => ({
                          key: key as keyof DownloadTask,
                          desc:
                            current.key === key
                              ? !current.desc
                              : ["created_at", "completed_at"].includes(key),
                        }))
                      }
                      style={{ cursor: "pointer", userSelect: "none" }}
                    >
                      {label}
                      {sort.key === key && (
                        <span style={{ fontSize: "10px", marginLeft: "4px", opacity: 0.85 }}>
                          {sort.desc ? "↓" : "↑"}
                        </span>
                      )}
                      {widthKey && (
                        <i
                          className="column-resizer"
                          onMouseDown={(event) =>
                            beginResize(widthKey, event)
                          }
                        />
                      )}
                    </span>
                  ))}
                  <span />
                </div>
                <div className="task-rows">
                  {loading ? (
                    <div className="center-state">
                      <LoaderCircle className="spin" />
                    </div>
                  ) : visible.length === 0 ? (
                    <EmptyState
                      filter={filter}
                      view={view}
                      onAdd={() => setNewOpen(true)}
                    />
                  ) : (
                    visible.map((task) => (
                      <TaskRow
                        key={task.id}
                        task={task}
                        showCompletedAt={showCompletedAt}
                        taskTagList={taskTags[task.id] ?? []}
                        selected={selected.has(task.id)}
                        notify={notify}
                        onSelect={() => {
                          setPrimaryTaskId(task.id);
                          setSelected((current) => {
                            const next = new Set(current);
                            next.has(task.id)
                              ? next.delete(task.id)
                              : next.add(task.id);
                            return next;
                          });
                        }}
                        onOpen={() =>
                          (task.status === "completed" ||
                            (task.task_kind === "bt" &&
                              task.downloaded_bytes > 0)) &&
                          void api
                            .openFile(task.id)
                            .catch((error) => notify(String(error), "error"))
                        }
                        onContext={(event) => {
                          event.preventDefault();
                          setPrimaryTaskId(task.id);
                          setContext({
                            x: event.clientX,
                            y: event.clientY,
                            id: task.id,
                          });
                          if (!selected.has(task.id))
                            setSelected(new Set([task.id]));
                        }}
                        onMouseDown={(taskItem, evt) => {
                          setPrimaryTaskId(taskItem.id);
                          handleTaskMouseDown(taskItem, evt);
                        }}
                        onCheckboxMouseDown={(evt) =>
                          handleCheckboxMouseDown(
                            task.id,
                            selected.has(task.id),
                            evt
                          )
                        }
                        onCheckboxMouseEnter={() =>
                          handleCheckboxMouseEnter(task.id)
                        }
                      />
                    ))
                  )}
                </div>
              </div>
            </div>
            {showDetails && (
              <Details
                task={activeTask}
                onClose={() => setShowDetails(false)}
                notify={notify}
                selectedCount={selected.size}
                onOpenProxySettings={() => {
                  setSettingsOpen(true);
                }}
                onOpenYouTubeModal={() =>
                  setYoutubeModalTaskId(activeTask?.id || "")
                }
                onOpenRefreshUrl={setRefreshUrlTarget}
                onTagsChanged={refreshTags}
              />
            )}
          </section>
          <BulkActionBar
            selectedCount={selected.size}
            onStartAll={() => void bulk("resume")}
            onPauseAll={() => void bulk("pause")}
            onDeleteRecords={() => void removeSelected(false)}
            onDeleteFiles={() => void removeSelected(true)}
            onDeselectAll={() => setSelected(new Set())}
          />
        </main>
        {dragOverlay &&
          !newOpen &&
          !settingsOpen &&
          !renameTarget &&
          !speedLimitTarget &&
          !aboutOpen &&
          !showCloseConfirm &&
          !context && (
            <div className="drop-overlay" aria-hidden="true">
              <div className="drop-overlay-card">
                <Download size={20} strokeWidth={2} />
                <span>{t("app.dropOverlay")}</span>
              </div>
            </div>
          )}
        {newOpen && (
          <NewTaskDialog
            settings={settings}
            allTasks={tasks}
            onClose={() => {
              setNewOpen(false);
              setInitialUrlFromClipboard("");
              setDroppedTorrent(null);
            }}
            onCreated={(created) => {
              setNewOpen(false);
              setInitialUrlFromClipboard("");
              setDroppedTorrent(null);
              const list = Array.isArray(created) ? created : [created];
              notify(t("toasts.addedTasks", { count: list.length }));
              if (list.length > 0) {
                setSelected(new Set(list.map((t) => t.id)));
              }
            }}
            defaultUrl={initialUrlFromClipboard}
            defaultTorrent={droppedTorrent}
            onLocateTask={(taskId) => {
              setNewOpen(false);
              setInitialUrlFromClipboard("");
              setDroppedTorrent(null);
              setSelected(new Set([taskId]));
              requestShowDetails(true);
              setView("main");
              setFilter("all");
            }}
            notify={notify}
          />
        )}
        {(() => {
          const contextTask = context
            ? tasks.find((t) => t.id === context.id)
            : undefined;
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
              onRefreshUrl={setRefreshUrlTarget}
              onDelete={(taskIds, deleteFile) =>
                void handleDeleteTasks(taskIds, deleteFile)
              }
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
              await api.updateTaskOptions(speedLimitTarget.id, {
                perTaskSpeedLimit: limitKb * 1024,
              });
              notify("限速已更新");
            }}
          />
        )}
        {globalToastLayer}
        {youtubeModalTaskId !== null && (
          <YouTubeCredentialsModal
            taskId={youtubeModalTaskId || undefined}
            onClose={() => setYoutubeModalTaskId(null)}
            notify={notify}
            onSuccessRetry={() => void refreshRef.current()}
          />
        )}
        {showCloseConfirm && (
          <CloseConfirmDialog
            onClose={() => setShowCloseConfirm(false)}
            onConfirm={handleCloseConfirm}
          />
        )}
        {aboutOpen && (
          <Modal
            title={t("app.about")}
            onClose={() => setAboutOpen(false)}
            style={{ width: "290px" }}
          >
            <div className="about-dialog-content">
              <div className="about-logo">
                <CatDownloadMark />
              </div>
              <h3>{t("app.nameFull")}</h3>
              <p className="about-version">{t("app.versionNumber")}</p>
              <p className="about-desc">
                {t("app.aboutDescLine1")}
                <br />
                {t("app.aboutDescLine2")}
              </p>
              <div className="about-links">
                <button
                  className="about-link-btn"
                  onClick={() =>
                    void openUrl(
                      "https://github.com/maobukeai/maobu-fetch"
                    )
                  }
                >
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

        {refreshUrlTarget && (
          <RefreshUrlDialog
            task={refreshUrlTarget}
            onClose={() => setRefreshUrlTarget(null)}
            onRefreshed={(updated) => {
              setRefreshUrlTarget(null);
              notify(t("dialogs.urlRefreshed") || "下载链接已更新", "ok");
            }}
            notify={notify}
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
