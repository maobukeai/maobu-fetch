import { useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import {
  AlertCircle, Archive, Check, CheckCircle2, ChevronDown, CirclePause, Copy,
  Download, ExternalLink, File, FileAudio, FileImage, FileText, Film, FolderOpen,
  Gauge, Globe2, Info, LoaderCircle, MonitorDown, MoreHorizontal, Network,
  PanelRightClose, PanelRightOpen, Pause, Play, Plus, RefreshCw, Search, Settings,
  ShieldCheck, SlidersHorizontal, Trash2, Unplug, Video, X,
} from "lucide-react";
import { api, isDesktop } from "./api";
import { Effect, getCurrentWindow } from "@tauri-apps/api/window";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import type {
  AppSettings, CollisionPolicy, DownloadTask, FilterKey, MediaProbeResult,
  NewTaskRequest, PairingInfo, TaskStatus, ToolComponent, ToolStatus,
} from "./types";

const statusText: Record<TaskStatus, string> = {
  queued: "等待中", downloading: "下载中", paused: "已暂停", completed: "已完成",
  failed: "失败", cancelled: "已取消", scheduled: "已计划", verifying: "校验中",
};
const nav: Array<[FilterKey, string, typeof Download]> = [
  ["all", "全部任务", Download], ["downloading", "正在下载", MonitorDown],
  ["queued", "等待中", Download], ["scheduled", "计划任务", Download],
  ["paused", "已暂停", CirclePause], ["completed", "已完成", CheckCircle2],
  ["failed", "失败", AlertCircle],
];
const categories: Array<[FilterKey, string, typeof Download]> = [
  ["video", "视频", Film], ["audio", "音频", FileAudio], ["images", "图片", FileImage],
  ["documents", "文档", FileText], ["archives", "压缩包", Archive], ["apps", "应用", File],
];
const defaults: AppSettings = {
  download_dir: "", concurrent_downloads: 3, connections_per_download: 8,
  speed_limit_kbps: 0, start_minimized: false, minimize_to_tray: true,
  close_to_tray: false, notifications: true, auto_start: false, theme: "system",
  frosted_glass: false,
  language: "zh-CN", intercept_browser_downloads: true, min_file_size_mb: 1,
  clipboard_monitor: false, proxy_mode: "system", proxy_url: "", proxy_username: "",
  proxy_password: "", user_agent: "MaobuFetch/0.5", default_collision_policy: "rename",
  max_retries: 3, retry_base_seconds: 2, verify_after_download: false,
  media_tool_auto_update: true,
  yt_dlp_path: "", ffmpeg_path: "", ffprobe_path: "",
  low_memory_mode: false,
  window_width: 1024,
  window_height: 720,
  auto_scale_ui: false,
};

function usesDarkTheme(theme: AppSettings["theme"]) {
  return theme === "dark" || (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
}

async function applyWindowAppearance(frostedGlass: boolean, dark: boolean) {
  document.documentElement.dataset.windowStyle = frostedGlass ? "frosted" : "solid";
  if (!isDesktop()) return;

  const appWindow = getCurrentWindow();
  if (frostedGlass) {
    await appWindow.setEffects({
      effects: [Effect.Acrylic],
      color: dark ? [24, 24, 27, 112] : [246, 248, 252, 104],
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
  const [settings, setSettings] = useState(defaults);
  const [loading, setLoading] = useState(true);
  const [fatal, setFatal] = useState<string>();
  const [filter, setFilter] = useState<FilterKey>("all");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<{ key: keyof DownloadTask; desc: boolean }>({ key: "created_at", desc: true });
  const [selected, setSelected] = useState(new Set<string>());
  const [showDetails, setShowDetails] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [categoriesExpanded, setCategoriesExpanded] = useState(true);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const [splash, setSplash] = useState(true);
  const [initialUrlFromClipboard, setInitialUrlFromClipboard] = useState("");
  const [toast, setToast] = useState<{ kind: "ok" | "error"; text: string }>();
  const [context, setContext] = useState<{ x: number; y: number; id: string }>();
  const [columnWidths, setColumnWidths] = useState<Record<string, number>>({});

  const refresh = async () => {
    try {
      setTasks(await api.list());
      if (isDesktop()) setSettings(await api.settings());
      setFatal(undefined);
    } catch (error) { setFatal(String(error)); }
    finally { setLoading(false); }
  };
  useEffect(() => {
    const handleContextMenu = (e: globalThis.MouseEvent) => e.preventDefault();
    document.addEventListener("contextmenu", handleContextMenu);

    const startTime = Date.now();
    void refresh().then(() => {
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
    return () => {
      document.removeEventListener("contextmenu", handleContextMenu);
      unlisten.forEach((item) => item());
    };
  }, []);
  useEffect(() => {
    const dark = usesDarkTheme(settings.theme);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    void applyWindowAppearance(settings.frosted_glass, dark).catch((error) => {
      document.documentElement.dataset.windowStyle = "solid";
      setToast({ kind: "error", text: `无法应用磨砂玻璃效果：${String(error)}` });
    });
  }, [settings.theme, settings.frosted_glass]);
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
          if (/^https?:\/\/[^\s]+$/i.test(text.trim())) {
            setInitialUrlFromClipboard(text.trim());
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

  const lastSelectedCount = useRef(0);
  useEffect(() => {
    const currentCount = selected.size;
    if (currentCount > lastSelectedCount.current && currentCount > 0) {
      setShowDetails(true);
    } else if (currentCount === 0) {
      setShowDetails(false);
    }
    lastSelectedCount.current = currentCount;
  }, [selected]);


  const visible = useMemo(() => tasks.filter((task) => {
    const category = categories.some(([key]) => key === filter) ? task.category === filter : true;
    const status = nav.some(([key]) => key === filter && key !== "all") ? task.status === filter : true;
    return category && status && `${task.file_name} ${task.url}`.toLowerCase().includes(search.toLowerCase());
  }).sort((a, b) => {
    const av = a[sort.key] ?? ""; const bv = b[sort.key] ?? "";
    const result = typeof av === "number" && typeof bv === "number" ? av - bv : String(av).localeCompare(String(bv));
    return sort.desc ? -result : result;
  }), [tasks, filter, search, sort]);
  const selectedTasks = tasks.filter((task) => selected.has(task.id));
  const selectedOne = selectedTasks.length === 1 ? selectedTasks[0] : undefined;
  const active = tasks.filter((task) => task.status === "downloading");
  const totalSpeed = active.reduce((sum, task) => sum + task.speed, 0);
  const notify = (text: string, kind: "ok" | "error" = "ok") => setToast({ text, kind });
  const bulk = async (action: string) => {
    try { await api.bulkAction([...selected], action); notify(action === "pause" ? "已暂停所选任务" : "任务已加入队列"); }
    catch (error) { notify(String(error), "error"); }
  };
  const removeSelected = async (deleteFile: boolean) => {
    try {
      for (const id of selected) await api.remove(id, deleteFile);
      setSelected(new Set()); notify(deleteFile ? "任务和文件已删除" : "任务记录已删除");
    } catch (error) { notify(String(error), "error"); }
  };
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
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") { event.preventDefault(); setNewOpen(true); }
      if (event.key === "Delete" && selected.size) void removeSelected(false);
      if (event.code === "Space" && selected.size && !event.repeat) {
        event.preventDefault(); void bulk(tasks.some((task) => selected.has(task.id) && task.status === "downloading") ? "pause" : "resume");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selected, tasks]);

  const titlebar = isDesktop() ? <Titlebar /> : null;

  if (settingsOpen) return (
    <div className="app-container">
      {titlebar}
      <SettingsPage value={settings} onChange={setSettings} onClose={() => setSettingsOpen(false)} notify={notify} />
      <WindowResizeHandles />
      {toast && <div className="toast"><span>{toast.kind === "ok" ? <Check size={14} /> : <AlertCircle size={14} />}</span>{toast.text}</div>}
      {showCloseConfirm && <CloseConfirmDialog onClose={() => setShowCloseConfirm(false)} onConfirm={handleCloseConfirm} />}
    </div>
  );
  const sectionTitle = [...nav, ...categories].find(([key]) => key === filter)?.[1] ?? "全部任务";
  return (
    <div className="app-container">
      {titlebar}
      <div className="app-frame">
        <aside className="nav-pane">
          <div className="brand"><div className="app-icon"><CatDownloadMark /></div><span><b>猫步下载器</b><small>Maobu Fetch</small></span></div>
          <button className="new-button" onClick={() => setNewOpen(true)}><Plus size={15} />新建任务</button>
          <div className="nav-scroll">
            <p className="nav-label">任务</p>
            {nav.map(([key, label, Icon]) => <button key={key} className={filter === key ? "nav-item active" : "nav-item"} onClick={() => setFilter(key)}><Icon size={14} /><span>{label}</span><small>{key === "all" ? tasks.length : tasks.filter((task) => task.status === key).length}</small></button>)}
            <p 
              className="nav-label interactive" 
              onClick={() => setCategoriesExpanded(!categoriesExpanded)}
            >
              <span>类型</span>
              <span className={`nav-label-chevron ${categoriesExpanded ? "" : "collapsed"}`}>
                <ChevronDown size={12} />
              </span>
            </p>
            {categoriesExpanded && (
              <div className="nav-grid">
                {categories.map(([key, label, Icon]) => <button key={key} className={filter === key ? "nav-item active" : "nav-item"} onClick={() => setFilter(key)}><Icon size={14} /><span>{label}</span><small>{tasks.filter((task) => task.category === key).length || ""}</small></button>)}
              </div>
            )}
          </div>
          <div className="nav-footer">
            <button className="nav-settings" onClick={() => setSettingsOpen(true)}><Settings size={15} /><span>设置</span></button>
            <div className="nav-status" onClick={() => setSettingsOpen(true)}>
              <i className={isDesktop() ? "status-dot online" : "status-dot offline"} />
              <span>↓ {formatBytes(totalSpeed)}/s · {active.length} 活动</span>
            </div>
          </div>
        </aside>
        <main className="workspace">
          <header className="titlebar" data-tauri-drag-region>
            <h1 data-tauri-drag-region>{sectionTitle}</h1>
            <label className="search-box"><Search size={14} /><input aria-label="搜索任务" value={search} onChange={(e) => setSearch(e.target.value)} placeholder="搜索名称或网址" />{search && <button onClick={() => setSearch("")}><X size={13} /></button>}</label>
            <div className="toolbar-actions">
              <button className="action-btn-standalone" onClick={() => setNewOpen(true)} title="新建任务"><Plus size={14} /></button>
              
              <div className="action-group">
                <button disabled={!selected.size} onClick={() => void bulk("resume")} title="开始任务"><Play size={14} /></button>
                <button disabled={!selected.size} onClick={() => void bulk("pause")} title="暂停任务"><Pause size={14} /></button>
                <button className="danger-action" disabled={!selected.size} onClick={() => void removeSelected(false)} title="删除记录"><Trash2 size={14} /></button>
              </div>

              <div className="action-group">
                <button disabled={!selectedOne || selectedOne.status !== "completed"} onClick={() => selectedOne && void api.openFile(selectedOne.id)} title="打开文件"><ExternalLink size={14} /></button>
                <button disabled={!selectedOne} onClick={() => selectedOne && void api.openFolder(selectedOne.id)} title="定位文件夹"><FolderOpen size={14} /></button>
              </div>

              <button className="action-btn-standalone" onClick={() => void refresh()} title="刷新列表"><RefreshCw size={14} /></button>
            </div>
            <button className="details-toggle" onClick={() => setShowDetails((value) => !value)} title="详情面板">{showDetails ? <PanelRightClose size={15} /> : <PanelRightOpen size={15} />}</button>
          </header>
          {fatal && <div className="error-banner"><Unplug size={16} /><span>无法连接下载内核：{fatal}</span><button onClick={() => void refresh()}>重试</button></div>}
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
              <div className="table-header"><label><input type="checkbox" aria-label="全选" checked={visible.length > 0 && visible.every((task) => selected.has(task.id))} onChange={() => setSelected(visible.every((task) => selected.has(task.id)) ? new Set() : new Set(visible.map((task) => task.id)))} /></label>{[["file_name","文件名",""],["total_bytes","大小","size"],["status","状态","status"],["connection_count","连接","connection"],["downloaded_bytes","进度","progress"],["speed","速度","speed"],["eta_seconds","剩余时间","eta"],["created_at","添加时间","created"]].map(([key,label,widthKey]) => <span key={key} onClick={() => setSort((current) => ({ key: key as keyof DownloadTask, desc: current.key === key ? !current.desc : key === "created_at" }))}>{label}{widthKey && <i className="column-resizer" onMouseDown={(event) => beginResize(widthKey, event)} />}</span>)}<span /></div>
              <div className="task-rows">{loading ? <div className="center-state"><LoaderCircle className="spin" /></div> : visible.length === 0 ? <EmptyState filter={filter} onAdd={() => setNewOpen(true)} /> : visible.map((task) => <TaskRow key={task.id} task={task} selected={selected.has(task.id)} onSelect={() => setSelected((current) => { const next = new Set(current); next.has(task.id) ? next.delete(task.id) : next.add(task.id); return next; })} onOpen={() => task.status === "completed" && void api.openFile(task.id)} onContext={(event) => { event.preventDefault(); setContext({ x: event.clientX, y: event.clientY, id: task.id }); if (!selected.has(task.id)) setSelected(new Set([task.id])); }} />)}</div>
            </div></div>
            {showDetails && <Details task={selectedOne} onClose={() => setShowDetails(false)} notify={notify} selectedCount={selected.size} />}
          </section>
        </main>
        {newOpen && <NewTaskDialog settings={settings} onClose={() => { setNewOpen(false); setInitialUrlFromClipboard(""); }} onCreated={(created) => {
          setNewOpen(false);
          setInitialUrlFromClipboard("");
          const list = Array.isArray(created) ? created : [created];
          notify(`已添加 ${list.length} 个任务`);
          if (list.length > 0) {
            setSelected(new Set(list.map((t) => t.id)));
            setShowDetails(true);
          }
        }} defaultUrl={initialUrlFromClipboard} />}
        {(() => {
          const contextTask = context ? tasks.find((t) => t.id === context.id) : undefined;
          return context && contextTask ? (
            <ContextMenu
              x={context.x}
              y={context.y}
              task={contextTask}
              close={() => setContext(undefined)}
              notify={notify}
            />
          ) : null;
        })()}
        {toast && <div className="toast"><span>{toast.kind === "ok" ? <Check size={14} /> : <AlertCircle size={14} />}</span>{toast.text}</div>}
        {showCloseConfirm && <CloseConfirmDialog onClose={() => setShowCloseConfirm(false)} onConfirm={handleCloseConfirm} />}
      </div>
      <WindowResizeHandles />
      {splash && (
        <div id="splash-screen" className="splash-overlay">
          <div className="splash-content">
            <div className="splash-logo">
              <CatDownloadMark />
            </div>
            <div className="splash-brand">
              <strong className="splash-title">猫步下载器</strong>
              <span className="splash-subtitle">Maobu Fetch</span>
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

function TaskRow({ task, selected, onSelect, onOpen, onContext }: { task: DownloadTask; selected: boolean; onSelect: () => void; onOpen: () => void; onContext: (event: MouseEvent) => void }) {
  const progress = task.total_bytes ? Math.min(100, task.downloaded_bytes / task.total_bytes * 100) : 0;
  return <div className={selected ? "task-row selected" : "task-row"} onDoubleClick={onOpen} onContextMenu={onContext}>
    <label><input type="checkbox" aria-label={`选择 ${task.file_name}`} checked={selected} onChange={onSelect} /></label>
    <div className="name-cell" onClick={onSelect}><FileIcon category={task.category} /><div><strong title={task.file_name}>{task.file_name}</strong><small title={task.url}>{hostOf(task.url)}</small></div></div>
    <span>{task.total_bytes ? formatBytes(task.total_bytes) : "—"}</span><span className={`task-status ${task.status}`}>{statusText[task.status]}</span><span className="connection-count">{task.status === "downloading" ? `${task.active_connections}/${task.connection_count}` : task.connection_count}<small> 路</small></span>
    <div className="progress-cell"><div><i style={{ width: `${progress}%` }} /></div><span>{task.status === "completed" ? "100%" : `${progress.toFixed(0)}%`}</span></div>
    <span>{task.status === "downloading" ? `${formatBytes(task.speed)}/s` : "—"}</span><span>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</span><span>{formatDate(task.created_at)}</span><button className="row-menu" onClick={(event) => { event.stopPropagation(); onContext(event); }}><MoreHorizontal size={15} /></button>
  </div>;
}
function CatDownloadMark() { return <svg viewBox="0 0 1024 1024" aria-hidden="true"><rect x="48" y="48" width="928" height="928" rx="220" fill="#f5f5f7" /><path d="M302 360 358 230l112 78c28-9 56-14 86-14s58 5 86 14l112-78 56 130v214c0 151-113 254-254 254S302 725 302 574V360Z" fill="#1d1d1f" /><path d="M556 392v218m-86-82 86 86 86-86" fill="none" stroke="#f5f5f7" strokeWidth="58" strokeLinecap="round" strokeLinejoin="round" /><path d="M445 694h222" fill="none" stroke="#0a84ff" strokeWidth="58" strokeLinecap="round" /><circle cx="428" cy="430" r="19" fill="#f5f5f7" /><circle cx="684" cy="430" r="19" fill="#f5f5f7" /><path d="M755 700c86 15 119-50 76-103" fill="none" stroke="#1d1d1f" strokeWidth="48" strokeLinecap="round" /></svg>; }
function FileIcon({ category }: { category: string }) { const Icon = category === "video" ? Film : category === "audio" ? FileAudio : category === "images" ? FileImage : category === "archives" ? Archive : category === "apps" ? File : FileText; return <span className={`file-type ${category}`}><Icon size={16} /></span>; }
function EmptyState({ filter, onAdd }: { filter: FilterKey; onAdd: () => void }) { return <div className="empty-state"><Download size={36} /><h2>{filter === "all" ? "还没有下载任务" : "此分类中没有任务"}</h2><p>添加链接，或从 Chrome / Edge 扩展发送下载。</p><button onClick={onAdd}>新建任务</button></div>; }

function Details({ task, onClose, notify, selectedCount }: { task?: DownloadTask; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void; selectedCount: number }) {
  if (!task) {
    return <aside className="details-pane">
      <div className="details-header">
        <h2>详情</h2>
        <button onClick={onClose} title="关闭"><X size={14} /></button>
      </div>
      <div className="details-scroll" style={{ justifyContent: "center", alignItems: "center", color: "var(--muted)", textAlign: "center", padding: "24px 16px", gap: "12px" }}>
        <Info size={32} strokeWidth={1.5} style={{ opacity: 0.4, marginBottom: "4px" }} />
        {selectedCount > 1 ? (
          <>
            <h3 style={{ fontSize: "12px", fontWeight: 600, color: "var(--text)", margin: 0 }}>已选择 {selectedCount} 个任务</h3>
            <p style={{ fontSize: "10px", margin: 0, lineHeight: 1.4 }}>选择单个任务以查看详细下载信息和分段连接状态。</p>
          </>
        ) : (
          <>
            <h3 style={{ fontSize: "12px", fontWeight: 600, color: "var(--text)", margin: 0 }}>未选择任务</h3>
            <p style={{ fontSize: "10px", margin: 0, lineHeight: 1.4 }}>在列表中选择一个下载任务以查看详细参数与分片连接。</p>
          </>
        )}
      </div>
    </aside>;
  }

  const action = async (value: string) => { try { await api.action(task.id, value); } catch (error) { notify(String(error), "error"); } };
  return <aside className="details-pane">
    <div className="details-header">
      <h2>{task.file_name}</h2>
      <button onClick={onClose} title="关闭"><X size={14} /></button>
    </div>
    <div className="details-scroll">
      <dl>
        <div><dt>状态</dt><dd>{statusText[task.status]}</dd></div>
        <div><dt>大小</dt><dd>{task.total_bytes ? formatBytes(task.total_bytes) : "—"}</dd></div>
        <div><dt>速度</dt><dd>{task.speed ? `${formatBytes(task.speed)}/s` : "—"}</dd></div>
        <div><dt>剩余时间</dt><dd>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</dd></div>
        <div><dt>来源</dt><dd>{hostOf(task.url)}</dd></div>
        <div><dt>保存位置</dt><dd>{task.destination}</dd></div>
        <div><dt>下载来源</dt><dd>{task.source}</dd></div>
        {task.checksum_sha256 && <div><dt>SHA-256 校验码</dt><dd title={task.checksum_sha256}>{task.checksum_sha256}</dd></div>}
      </dl>

      {task.error && <div className="task-error">{task.error}</div>}

      {task.segments.length > 0 && (
        <div className="segment-panel">
          <div className="segment-title">并发连接（{task.active_connections} 活动 / 上限 {task.connection_count}） · {task.segments.length} 个分片</div>
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
        {task.status === "downloading" ? (
          <button onClick={() => void action("pause")}><Pause size={13} />暂停下载</button>
        ) : (
          !["completed", "cancelled"].includes(task.status) && <button onClick={() => void action("resume")}><Play size={13} />继续下载</button>
        )}
        <button onClick={() => void api.openFolder(task.id)}><FolderOpen size={13} />打开目录</button>
        {task.status === "completed" && (
          <button onClick={async () => {
            try {
              const hash = await api.verify(task.id);
              notify(`校验完成：${hash.slice(0, 12)}…`);
            } catch (error) {
              notify(String(error), "error");
            }
          }}><ShieldCheck size={13} />校验文件</button>
        )}
      </div>
    </div>
  </aside>;
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

function NewTaskDialog({ settings, onClose, onCreated, defaultUrl }: { settings: AppSettings; onClose: () => void; onCreated: (tasks: DownloadTask | DownloadTask[]) => void; defaultUrl?: string }) {
  const [urls, setUrls] = useState(defaultUrl || ""); const [destination, setDestination] = useState(settings.download_dir);
  const [fileName, setFileName] = useState(() => {
    if (defaultUrl) {
      const lines = defaultUrl.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
      if (lines.length === 1) {
        return extractFileNameFromUrl(lines[0]);
      }
    }
    return "";
  }); const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false); const [error, setError] = useState<string>();
  const [schedule, setSchedule] = useState(""); const [policy, setPolicy] = useState<CollisionPolicy>(settings.default_collision_policy);
  const [referer, setReferer] = useState(""); const [cookie, setCookie] = useState(""); const [authorization, setAuthorization] = useState("");
  const [checksum, setChecksum] = useState(""); const [limit, setLimit] = useState(0);
  const [connections, setConnections] = useState(settings.connections_per_download);
  const [media, setMedia] = useState<MediaProbeResult>(); const [format, setFormat] = useState("");
  const [toolStatus, setToolStatus] = useState<ToolStatus>();
  useEffect(() => { let unlisten: (() => void) | undefined; void api.toolStatus().then(setToolStatus); void api.subscribeMediaTools(setToolStatus).then((value) => { unlisten = value; }); return () => unlisten?.(); }, []);
  const lines = urls.split(/\r?\n/).map((value) => value.trim()).filter(Boolean);
  const probe = async () => { setBusy(true); setError(undefined); try { const result = await api.probeMedia(lines[0]); if (result.drm) throw new Error("检测到 DRM 保护，猫步下载器不处理此内容"); setMedia(result); const direct = result.formats.filter((item) => item.has_video && item.has_audio && !item.requires_ffmpeg).sort((a, b) => (b.height ?? 0) - (a.height ?? 0)); const video = result.formats.filter((item) => item.has_video && !item.requires_ffmpeg).sort((a, b) => (b.height ?? 0) - (a.height ?? 0)); const selected = direct[0] ?? video[0] ?? result.formats[0]; setFormat(selected?.id ?? ""); if (!fileName) setFileName(`${safeDisplayName(result.title)}.mp4`); } catch (reason) { const text = String(reason); if (text.includes("MEDIA_YT_DLP_MISSING")) setToolStatus(await api.toolStatus()); else setError(text); } finally { setBusy(false); } };
  const submit = async () => {
    if (!lines.length) return; setBusy(true); setError(undefined);
    const headers: Record<string, string> = {}; if (referer) headers.Referer = referer; if (cookie) headers.Cookie = cookie; if (authorization) headers.Authorization = authorization;
    const selectedFormat = media?.formats.find((item) => item.id === format);
    if (selectedFormat?.requires_ffmpeg && !toolStatus?.ffmpeg_available) {
      setError("当前最高画质需要先安装 FFmpeg 高清合并组件");
      setBusy(false);
      return;
    }
    const template: Omit<NewTaskRequest, "url"> = { file_name: fileName || undefined, destination, headers, scheduled_at: schedule ? new Date(schedule).getTime() : undefined, priority: 0, expected_checksum: checksum || undefined, source: "desktop", per_task_speed_limit: limit * 1024, collision_policy: policy, connection_count: connections, media: media ? { extractor: media.extractor, format_id: format, format_label: selectedFormat?.label, subtitles: [], thumbnail: media.thumbnail, requires_ffmpeg: selectedFormat?.requires_ffmpeg } : undefined };
    try {
      if (lines.length === 1) {
        const task = await api.add({ url: lines[0], ...template });
        onCreated(task);
      } else {
        const tasks = await api.addBatch(lines, template);
        onCreated(tasks);
      }
    } catch (reason) { setError(String(reason)); setBusy(false); }
  };
  return (
    <Modal title="新建下载任务" onClose={onClose}>
      <div className="new-task-form">
        <div className="form-section">
          <label className="form-field">
            <div className="form-label-bar">
              <span>下载链接（每行一个）</span>
              {lines.length > 0 && (
                <span className="form-label-counter">
                  已检测到 {lines.length} 个链接
                </span>
              )}
            </div>
            <textarea
              autoFocus
              value={urls}
              onChange={(e) => {
                const val = e.target.value;
                setUrls(val);
                setMedia(undefined);
                const currentLines = val.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
                if (currentLines.length === 1) {
                  const name = extractFileNameFromUrl(currentLines[0]);
                  if (name) {
                    setFileName((prev) => prev === "" ? name : prev);
                  }
                } else if (currentLines.length === 0) {
                  setFileName("");
                }
              }}
              placeholder="https://example.com/file.zip"
            />
          </label>
        </div>

        <div className="form-group-row">
          <label className="form-field grow">
            <span>保存位置</span>
            <div className="input-group">
              <input
                value={destination}
                onChange={(e) => setDestination(e.target.value)}
              />
              <button
                className="input-button primary-border"
                onClick={async () => {
                  const path = await pickPath({
                    directory: true,
                    multiple: false,
                    defaultPath: destination,
                  });
                  if (typeof path === "string") setDestination(path);
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
              <span className="field-label-value">{connections} 路并发</span>
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
                  value={fileName}
                  onChange={(e) => setFileName(e.target.value)}
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
            <div className="media-format-select-row">
              <select value={format} onChange={(e) => setFormat(e.target.value)}>
                {media.formats
                  .filter((item) => item.has_video || item.has_audio)
                  .map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.label}
                      {item.file_size ? ` (${formatBytes(item.file_size)})` : ""}
                      {!item.requires_ffmpeg && item.has_video && item.has_audio ? " · 轻量单文件" : ""}
                    </option>
                  ))}
              </select>
            </div>
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
              <Field label="预期文件 SHA-256">
                <input
                  value={checksum}
                  onChange={(e) => setChecksum(e.target.value)}
                  placeholder="用于校验文件完整性"
                />
              </Field>
            </div>
          </div>
        )}

        {error && <div className="inline-error">{error}</div>}

        <div className="dialog-actions new-task-actions">
          <button className="cancel-btn" onClick={onClose}>
            取消
          </button>
          <button
            className="primary confirm-btn"
            disabled={busy || !lines.length}
            onClick={() => void submit()}
          >
            {busy ? "正在提交任务..." : "开始下载"}
          </button>
        </div>
      </div>
    </Modal>
  );
}

type SettingsSection = "general" | "download" | "network" | "browser" | "media" | "appearance" | "advanced" | "about";
function SettingsPage({ value, onChange, onClose, notify }: { value: AppSettings; onChange: (value: AppSettings) => void; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void }) {
  const appWindow = useMemo(() => isDesktop() ? getCurrentWindow() : null, []);
  const [draft, setDraft] = useState(value); const [section, setSection] = useState<SettingsSection>("general");
  const [pair, setPair] = useState<PairingInfo>(); const [tools, setTools] = useState<ToolStatus>();
  const set = <K extends keyof AppSettings>(key: K, val: AppSettings[K]) => setDraft((item) => ({ ...item, [key]: val }));

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
  useEffect(() => { let unlisten: (() => void) | undefined; if (section === "browser") void api.pairing().then(setPair); if (section === "media") { void api.toolStatus().then(setTools); void api.subscribeMediaTools(setTools).then((value) => { unlisten = value; }); } return () => unlisten?.(); }, [section]);
  useEffect(() => {
    const dark = usesDarkTheme(draft.theme);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    void applyWindowAppearance(draft.frosted_glass, dark).catch((error) => {
      document.documentElement.dataset.windowStyle = "solid";
      notify(`无法预览磨砂玻璃效果：${String(error)}`, "error");
    });
  }, [draft.theme, draft.frosted_glass]);
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
      const dark = usesDarkTheme(finalSettings.theme);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
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
  const save = async () => { try { await api.saveSettings(draft); hasSaved.current = true; onChange(draft); notify("设置已保存"); onClose(); } catch (error) { notify(String(error), "error"); } };
  const items: Array<[SettingsSection, string, typeof Settings]> = [["general","常规",Settings],["download","下载",Download],["network","网络",Network],["browser","浏览器",Globe2],["media","媒体",Video],["appearance","外观",SlidersHorizontal],["advanced","高级",Info],["about","关于",Info]];
  return <div className="settings-page"><aside className="nav-pane"><div className="brand" data-tauri-drag-region>设置</div>{items.map(([key,label,Icon]) => <button key={key} className={section === key ? "nav-item active" : "nav-item"} onClick={() => setSection(key)}><Icon size={15} /><span>{label}</span></button>)}</aside><main className="settings-body" data-tauri-drag-region><div className="settings-title" data-tauri-drag-region><h1 data-tauri-drag-region>{items.find(([key]) => key === section)?.[1]}</h1></div><div className="settings-content">
    {section === "general" && <SettingsGroup title="应用行为"><div className="settings-group-content"><Toggle label="启动时最小化" checked={draft.start_minimized} onChange={(v) => set("start_minimized", v)} /><Toggle label="最小化到托盘" checked={draft.minimize_to_tray} onChange={(v) => set("minimize_to_tray", v)} /><Toggle label="关闭时驻留托盘" checked={draft.close_to_tray} onChange={(v) => set("close_to_tray", v)} /><Toggle label="下载完成通知" checked={draft.notifications} onChange={(v) => set("notifications", v)} /><Toggle label="监视剪贴板链接" checked={draft.clipboard_monitor} onChange={(v) => set("clipboard_monitor", v)} /></div></SettingsGroup>}
    {section === "download" && <SettingsGroup title="保存与性能"><div className="settings-group-content"><SettingRow label="默认下载目录"><input value={draft.download_dir} onChange={(e) => set("download_dir", e.target.value)} /></SettingRow><SettingRow label="文件重名"><div className="fluent-segmented-control settings-segmented"><button type="button" className={draft.default_collision_policy === "rename" ? "active" : ""} onClick={() => set("default_collision_policy", "rename")}>自动重命名</button><button type="button" className={draft.default_collision_policy === "overwrite" ? "active" : ""} onClick={() => set("default_collision_policy", "overwrite")}>覆盖</button><button type="button" className={draft.default_collision_policy === "skip" ? "active" : ""} onClick={() => set("default_collision_policy", "skip")}>跳过</button></div></SettingRow><Toggle label="低内存模式（1 个任务、每任务最多 2 路连接）" checked={draft.low_memory_mode} onChange={(v) => set("low_memory_mode", v)} /><SettingRow label="同时下载任务"><input type="number" min="1" max="16" value={draft.concurrent_downloads} onChange={(e) => set("concurrent_downloads", +e.target.value)} /></SettingRow><SettingRow label={`每任务连接数 (${draft.connections_per_download} 路)`}><div className="settings-slider-wrapper"><input type="range" min="0" max="5" step="1" value={[1, 2, 4, 8, 16, 32].indexOf(draft.connections_per_download)} onChange={(e) => { const values = [1, 2, 4, 8, 16, 32]; set("connections_per_download", values[+e.target.value]); }} className="fluent-slider" /><div className="slider-ticks"><span>1</span><span>2</span><span>4</span><span>8</span><span>16</span><span>32</span></div></div></SettingRow><SettingRow label="全局限速（KB/s）"><input type="number" min="0" value={draft.speed_limit_kbps} onChange={(e) => set("speed_limit_kbps", +e.target.value)} /></SettingRow><Toggle label="完成后计算 SHA-256" checked={draft.verify_after_download} onChange={(v) => set("verify_after_download", v)} /></div><p className="settings-note">开启后使用更小的合并缓冲区和连接池；不会改写上述并发偏好，关闭后自动恢复。已建立的连接会安全完成。</p></SettingsGroup>}
    {section === "network" && <SettingsGroup title="代理与重试"><div className="settings-group-content"><SettingRow label="代理模式"><select value={draft.proxy_mode} onChange={(e) => set("proxy_mode", e.target.value as AppSettings["proxy_mode"])}><option value="system">跟随系统</option><option value="none">不使用代理</option><option value="manual">手动代理</option></select></SettingRow>{draft.proxy_mode === "manual" && <><SettingRow label="代理地址"><input value={draft.proxy_url} onChange={(e) => set("proxy_url", e.target.value)} /></SettingRow><SettingRow label="用户名"><input value={draft.proxy_username} onChange={(e) => set("proxy_username", e.target.value)} /></SettingRow><SettingRow label="密码"><input type="password" value={draft.proxy_password} onChange={(e) => set("proxy_password", e.target.value)} /></SettingRow></>}<SettingRow label="最大重试次数"><input type="number" min="0" max="10" value={draft.max_retries} onChange={(e) => set("max_retries", +e.target.value)} /></SettingRow></div></SettingsGroup>}
    {section === "browser" && <><SettingsGroup title="下载接管"><div className="settings-group-content"><Toggle label="允许浏览器扩展接管下载" checked={draft.intercept_browser_downloads} onChange={(v) => set("intercept_browser_downloads", v)} /><SettingRow label="最小文件大小（MB）"><input type="number" min="0" value={draft.min_file_size_mb} onChange={(e) => set("min_file_size_mb", +e.target.value)} /></SettingRow></div></SettingsGroup><SettingsGroup title="安全配对">{pair ? <div className="pair-card"><p>在扩展中输入一次性配对码（10 分钟有效）</p><div className="pair-code-wrapper"><code>{pair.code}</code><button className="copy-code-btn" onClick={() => { void navigator.clipboard.writeText(pair.code); notify("配对码已复制到剪贴板"); }} title="复制配对码"><Copy size={13} /><span>复制</span></button></div>{pair.paired_extension && <p>已配对：{pair.paired_extension.slice(0, 16)}…</p>}<div className="maintenance"><button onClick={() => void api.rotatePairing().then(setPair)}>更换配对码</button>{pair.paired_extension && <button onClick={() => void api.revokePairing().then(() => api.pairing().then(setPair))}>撤销配对</button>}</div></div> : <LoaderCircle className="spin" />}</SettingsGroup></>}
    {section === "media" && <SettingsGroup title="媒体组件"><p className="settings-note">按“自定义路径 → 应用安装 → Windows PATH”顺序查找组件。外部组件只会被引用，猫步下载器不会复制、更新或删除它们。</p>{tools ? <MediaToolsCard status={tools} onStatus={setTools} /> : <LoaderCircle className="spin" />}<MediaPathSettings value={draft} onChange={(patch) => setDraft((current) => ({ ...current, ...patch }))} /><div className="settings-group-content"><Toggle label="自动检查媒体工具更新" checked={draft.media_tool_auto_update} onChange={(v) => set("media_tool_auto_update", v)} /></div></SettingsGroup>}
    {section === "appearance" && <SettingsGroup title="主题与窗口"><div className="settings-group-content"><SettingRow label="应用主题"><select value={draft.theme} onChange={(e) => set("theme", e.target.value as AppSettings["theme"])}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></select></SettingRow><Toggle label="磨砂玻璃" checked={draft.frosted_glass} onChange={(v) => set("frosted_glass", v)} /><SettingRow label="窗口大小"><div className="window-size-setting-row"><input type="number" placeholder="宽度 (如 800)" value={draft.window_width || ""} onChange={(e) => changeWidth(e.target.value ? +e.target.value : undefined)} className="window-size-input" /><span>×</span><input type="number" placeholder="高度 (如 600)" value={draft.window_height || ""} onChange={(e) => changeHeight(e.target.value ? +e.target.value : undefined)} className="window-size-input" /><select value={draft.window_width && draft.window_height ? `${draft.window_width}x${draft.window_height}` : ""} onChange={(e) => { if (!e.target.value) return; const [w, h] = e.target.value.split("x").map(Number); set("window_width", w); set("window_height", h); applyTemporarySize(w, h); }} className="window-size-preset-select"><option value="">选择常用预设...</option><option value="800x600">800 × 600 (迷你紧凑)</option><option value="960x640">960 × 640 (精致比例)</option><option value="1024x720">1024 × 720 (默认标准)</option><option value="1120x760">1120 × 760 (舒适格局)</option><option value="1280x800">1280 × 800 (高效宽屏)</option><option value="1440x900">1440 × 900 (专业超宽)</option></select></div></SettingRow><Toggle label="自适应缩放" checked={draft.auto_scale_ui || false} onChange={(v) => set("auto_scale_ui", v)} /></div><p className="settings-note">磨砂玻璃使用 Windows 10/11 原生 Acrylic 材质；关闭后恢复为不透明窗口。</p></SettingsGroup>}
    {section === "advanced" && <SettingsGroup title="维护"><div className="maintenance"><button onClick={() => void api.clearHistory(false).then(() => notify("已清理取消的任务"))}>清理取消任务</button><button onClick={() => void api.clearHistory(true).then(() => notify("下载历史已清理"))}>清理完成和取消任务</button></div></SettingsGroup>}
    {section === "about" && (
      <SettingsGroup title="关于猫步下载器">
        <div style={{ display: "flex", flexDirection: "column", gap: "16px", padding: "10px 0" }}>
          <div style={{ display: "flex", alignItems: "center", gap: "16px" }}>
            <div style={{ width: "64px", height: "64px", flexShrink: 0 }}>
              <CatDownloadMark />
            </div>
            <div>
              <h2 style={{ margin: 0, fontSize: "16px", fontWeight: 700, color: "var(--text)" }}>猫步下载器 (Maobu Fetch)</h2>
              <p style={{ margin: "4px 0 0", fontSize: "11px", color: "var(--muted)" }}>版本 0.5.0 (Beta)</p>
            </div>
          </div>

          <div style={{ borderTop: "1px solid var(--border)", paddingTop: "14px" }}>
            <h3 style={{ margin: "0 0 6px", fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>作者 / 开发团队</h3>
            <p style={{ margin: 0, fontSize: "11px", color: "var(--muted)", lineHeight: 1.5 }}>
              猫步可爱 (maobukeai)
            </p>
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

          <div style={{ borderTop: "1px solid var(--border)", paddingTop: "14px", display: "flex", flexDirection: "column", gap: "10px" }}>
            <h3 style={{ margin: "0", fontSize: "12px", fontWeight: 600, color: "var(--text)" }}>开源项目主页</h3>
            <p style={{ margin: 0, fontSize: "11px", color: "var(--muted)", lineHeight: 1.5 }}>
              本下载管理器属于开源共享项目。欢迎访问 GitHub 主页获取最新构建版本以更新软件，或参与社区提交代码改进：
            </p>
            <div>
              <button 
                className="dialog-actions-btn primary" 
                style={{ 
                  display: "inline-flex", 
                  alignItems: "center", 
                  gap: "6px", 
                  height: "28px", 
                  padding: "0 14px",
                  fontSize: "11px",
                  fontWeight: 500,
                  cursor: "pointer",
                  borderRadius: "6px",
                  border: "none",
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
                访问 GitHub 获取最新更新
              </button>
            </div>
          </div>
        </div>
      </SettingsGroup>
    )}
    <div className="dialog-actions settings-actions"><button onClick={onClose}>取消</button><button className="primary" onClick={() => void save()}>保存设置</button></div>
  </div></main></div>;
}

function SettingsGroup({ title, children }: { title: string; children: ReactNode }) { return <section className="settings-group"><h2>{title}</h2><div>{children}</div></section>; }
function SettingRow({ label, children }: { label: string; children: ReactNode }) { return <label className="setting-row"><div><strong>{label}</strong></div>{children}</label>; }
function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) { return <label className="setting-row"><div><strong>{label}</strong></div><input className="toggle" type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} /></label>; }
function Field({ label, children }: { label: string; children: ReactNode }) { return <label className="form-field"><span>{label}</span>{children}</label>; }
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
      <div><strong>自动使用系统已有组件</strong><small>检查 PATH、Python、Scoop、WinGet 等常见位置，并将绝对路径填入下方</small></div>
      <button className="input-button" disabled={detecting} onClick={() => void detectSystemTools()}>{detecting ? "检测中…" : "自动检测"}</button>
    </div>
    {detectionMessage && <p className="media-detect-result" role="status">{detectionMessage}</p>}
    <SettingRow label="自定义 yt-dlp.exe"><div className="input-group"><input value={value.yt_dlp_path} onChange={(event) => onChange({ yt_dlp_path: event.target.value })} placeholder="留空则自动检测" /><button className="input-button" onClick={() => void chooseYtDlp()}>选择文件</button>{value.yt_dlp_path && <button className="input-button" onClick={() => onChange({ yt_dlp_path: "" })}>清除</button>}</div></SettingRow>
    <SettingRow label="自定义 ffmpeg.exe"><div className="input-group"><input value={value.ffmpeg_path} onChange={(event) => onChange({ ffmpeg_path: event.target.value })} placeholder="留空则自动检测" /><button className="input-button" onClick={() => void chooseFfmpeg()}>选择文件</button>{value.ffmpeg_path && <button className="input-button" onClick={() => onChange({ ffmpeg_path: "", ffprobe_path: "" })}>清除</button>}</div></SettingRow>
    {value.ffmpeg_path && <SettingRow label="配套 ffprobe.exe"><input value={value.ffprobe_path} onChange={(event) => onChange({ ffprobe_path: event.target.value })} /></SettingRow>}
  </div>;
}
function MediaToolsCard({ status, onStatus, compact = false, required }: { status: ToolStatus; onStatus: (value: ToolStatus) => void; compact?: boolean; required?: ToolComponent }) {
  const components: ToolComponent[] = required ? [required] : ["yt-dlp", "ffmpeg"];
  return <div className={compact ? "media-tools-stack compact" : "media-tools-stack"}>
    {components.map((component) => <MediaToolComponentCard key={component} component={component} status={status} onStatus={onStatus} compact={compact} />)}
  </div>;
}

function MediaToolComponentCard({ component, status, onStatus, compact }: { component: ToolComponent; status: ToolStatus; onStatus: (value: ToolStatus) => void; compact: boolean }) {
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
  const sourceLabel = source === "custom" ? "自定义路径" : source === "system" ? "系统 PATH" : source === "bundled" ? "应用安装" : "未安装";
  const title = isYtDlp ? "yt-dlp 基础媒体组件" : "FFmpeg 高清合并组件";
  const description = isYtDlp ? "媒体分析、单文件视频和音频下载" : "最高画质音视频合并、转码与格式处理";
  const progress = status.total_bytes ? Math.min(100, status.downloaded_bytes / status.total_bytes * 100) : 0;
  const install = async () => { try { await api.installMediaTool(component); onStatus(await api.toolStatus()); } catch (error) { onStatus({ ...status, active_component: component, state: "failed", error: String(error) }); } };
  const remove = async () => { try { await api.removeMediaTool(component); onStatus(await api.toolStatus()); } catch (error) { onStatus({ ...status, active_component: component, state: "failed", error: String(error) }); } };
  return <div className={compact ? "media-tools-card compact" : "media-tools-card"}>
    <div className="media-tools-card-main">
      <div className="tool-summary">
        <span className={`tool-state ${phase}`}>{available && !active ? <Check size={14} /> : active ? <LoaderCircle className="spin" size={14} /> : <Video size={14} />}</span>
        <div><strong>{title}</strong><small>{description} · {version}{available ? source === "bundled" ? ` · 应用占用 ${formatBytes(installedBytes)}` : ` · 使用${sourceLabel}` : ` · 下载约 ${formatBytes(downloadBytes)} · 安装约 ${formatBytes(installEstimate)}`}</small></div>
      </div>
      <div className="tool-actions">
        {active ? <button onClick={() => void api.cancelMediaTools()}>取消安装</button> : available && source === "bundled" ? <button className="danger" disabled={someInstallActive} onClick={() => void remove()}>卸载</button> : available ? <button disabled>已使用外部组件</button> : <button className="primary" disabled={someInstallActive} onClick={() => void install()}>下载并安装</button>}
      </div>
    </div>
    {active && <div className="tool-progress"><div><i style={{ width: `${progress}%` }} /></div><span>{status.state === "verifying" ? "正在校验 SHA-256" : status.state === "extracting" ? "正在安全解压" : `${formatBytes(status.downloaded_bytes)} / ${formatBytes(status.total_bytes)}`}</span></div>}
    {operationForThis && status.error && <p className="tool-error">{status.error}</p>}
  </div>;
}
function ContextMenu({ x, y, task, close, notify }: { x: number; y: number; task: DownloadTask; close: () => void; notify: (text: string, kind?: "ok" | "error") => void }) {
  const action = async (value: string) => {
    try {
      await api.action(task.id, value);
      close();
    } catch (error) {
      notify(String(error), "error");
    }
  };
  const safeX = Math.min(x, window.innerWidth - 170 - 12);
  const safeY = Math.min(y, window.innerHeight - 195 - 12);
  return (
    <div className="context-menu" style={{ left: safeX, top: safeY }} onClick={(e) => e.stopPropagation()}>
      {task.status === "downloading" ? <button onClick={() => void action("pause")}><Pause size={13} />暂停</button> : !["completed","cancelled"].includes(task.status) && <button onClick={() => void action("resume")}><Play size={13} />开始 / 继续</button>}
      <button onClick={() => void api.openFolder(task.id).then(close)}><FolderOpen size={13} />打开文件夹</button>
      <button onClick={() => void navigator.clipboard.writeText(task.url).then(() => { notify("链接已复制"); close(); })}><Copy size={13} />复制链接</button>
      {task.status === "completed" && <button onClick={() => void api.verify(task.id).then(() => { notify("文件校验完成"); close(); })}><ShieldCheck size={13} />校验 SHA-256</button>}
      <button className="danger" onClick={() => void api.remove(task.id, false).then(close)}><Trash2 size={13} />删除记录</button>
      <button className="danger" onClick={() => void api.remove(task.id, true).then(close)}><Trash2 size={13} />删除记录和文件</button>
    </div>
  );
}
function Modal({ title, onClose, wide, children, style }: { title: string; onClose: () => void; wide?: boolean; children: ReactNode; style?: CSSProperties }) { return <div className="modal-layer" onMouseDown={onClose}><section className={wide ? "dialog wide" : "dialog"} style={style} onMouseDown={(e) => e.stopPropagation()}><div className="settings-title"><h2>{title}</h2></div>{children}</section></div>; }
function formatBytes(value: number) { if (!value) return "0 B"; const units = ["B","KB","MB","GB","TB"]; const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1); return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`; }
function formatDuration(seconds: number) { if (seconds < 60) return `${seconds} 秒`; if (seconds < 3600) return `${Math.ceil(seconds / 60)} 分钟`; return `${Math.floor(seconds / 3600)} 小时 ${Math.ceil(seconds % 3600 / 60)} 分`; }
function formatDate(value: number) { return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(new Date(value)); }
function hostOf(url: string) { try { return new URL(url).host; } catch { return url; } }
function safeDisplayName(value: string) { return value.replace(/[<>:"/\\|?*]/g, "_").slice(0, 120); }

function CloseConfirmDialog({ onClose, onConfirm }: { onClose: () => void; onConfirm: (action: "tray" | "exit", remember: boolean) => void }) {
  const [remember, setRemember] = useState(false);

  return (
    <Modal title="关闭提示" onClose={onClose} style={{ width: "380px" }}>
      <div className="new-task-form" style={{ gap: "16px", padding: "4px 0 0" }}>
        <p style={{ margin: 0, fontSize: "12px", color: "var(--text)", lineHeight: 1.5 }}>
          关闭主窗口后，是否允许猫步下载器继续在后台运行？
        </p>
        
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginTop: "8px", width: "100%", gap: "12px" }}>
          <label style={{ display: "flex", alignItems: "center", gap: "6px", fontSize: "11px", color: "var(--muted)", cursor: "pointer", userSelect: "none" }}>
            <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} style={{ width: "13px", height: "13px", accentColor: "var(--accent)" }} />
            <span>记住选择，不再提示</span>
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
              直接退出
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
              后台运行
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
