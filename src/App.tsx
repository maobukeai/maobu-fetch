import { useEffect, useMemo, useState, type MouseEvent, type ReactNode } from "react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import {
  AlertCircle, Archive, Check, CheckCircle2, ChevronDown, CirclePause, Copy,
  Download, ExternalLink, File, FileAudio, FileImage, FileText, Film, FolderOpen,
  Gauge, Globe2, Info, LoaderCircle, MonitorDown, MoreHorizontal, Network,
  PanelRightClose, PanelRightOpen, Pause, Play, Plus, RefreshCw, Search, Settings,
  ShieldCheck, SlidersHorizontal, Trash2, Unplug, Video, X,
} from "lucide-react";
import { api, isDesktop } from "./api";
import type {
  AppSettings, CollisionPolicy, DownloadTask, FilterKey, MediaProbeResult,
  NewTaskRequest, PairingInfo, TaskStatus, ToolStatus,
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
  language: "zh-CN", intercept_browser_downloads: true, min_file_size_mb: 1,
  clipboard_monitor: false, proxy_mode: "system", proxy_url: "", proxy_username: "",
  proxy_password: "", user_agent: "LumaGet/0.2", default_collision_policy: "rename",
  max_retries: 3, retry_base_seconds: 2, verify_after_download: false,
  media_tool_auto_update: true,
};

export default function App() {
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [settings, setSettings] = useState(defaults);
  const [loading, setLoading] = useState(true);
  const [fatal, setFatal] = useState<string>();
  const [filter, setFilter] = useState<FilterKey>("all");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<{ key: keyof DownloadTask; desc: boolean }>({ key: "created_at", desc: true });
  const [selected, setSelected] = useState(new Set<string>());
  const [showDetails, setShowDetails] = useState(true);
  const [newOpen, setNewOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [toast, setToast] = useState<{ kind: "ok" | "error"; text: string }>();
  const [context, setContext] = useState<{ x: number; y: number; id: string }>();

  const refresh = async () => {
    try {
      setTasks(await api.list());
      if (isDesktop()) setSettings(await api.settings());
      setFatal(undefined);
    } catch (error) { setFatal(String(error)); }
    finally { setLoading(false); }
  };
  useEffect(() => {
    void refresh();
    let unlisten: Array<() => void> = [];
    void api.subscribe((event) => {
      if ("removed" in event) setTasks((items) => items.filter((task) => task.id !== event.removed));
      else setTasks((items) => items.some((task) => task.id === event.task.id)
        ? items.map((task) => task.id === event.task.id ? event.task : task)
        : [event.task, ...items]);
    }).then((items) => { unlisten = items; });
    return () => unlisten.forEach((item) => item());
  }, []);
  useEffect(() => {
    const dark = settings.theme === "dark" || (settings.theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [settings.theme]);
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

  if (settingsOpen) return <SettingsPage value={settings} onChange={setSettings} onClose={() => setSettingsOpen(false)} notify={notify} />;
  const sectionTitle = [...nav, ...categories].find(([key]) => key === filter)?.[1] ?? "全部任务";
  return <div className="app-frame">
    <aside className="nav-pane">
      <div className="brand"><div className="app-icon"><Download size={18} /></div><span>LumaGet</span></div>
      <button className="new-button" onClick={() => setNewOpen(true)}><Plus size={15} />新建任务</button>
      <div className="nav-scroll">
        <p className="nav-label">任务</p>
        {nav.map(([key, label, Icon]) => <button key={key} className={filter === key ? "nav-item active" : "nav-item"} onClick={() => setFilter(key)}><Icon size={15} /><span>{label}</span><small>{key === "all" ? tasks.length : tasks.filter((task) => task.status === key).length}</small></button>)}
        <p className="nav-label">类型</p>
        {categories.map(([key, label, Icon]) => <button key={key} className={filter === key ? "nav-item active" : "nav-item"} onClick={() => setFilter(key)}><Icon size={15} /><span>{label}</span><small>{tasks.filter((task) => task.category === key).length || ""}</small></button>)}
      </div>
      <button className="nav-settings" onClick={() => setSettingsOpen(true)}><Settings size={15} /><span>设置</span></button>
    </aside>
    <main className="workspace">
      <header className="titlebar"><h1>{sectionTitle}</h1><label className="search-box"><Search size={14} /><input aria-label="搜索任务" value={search} onChange={(e) => setSearch(e.target.value)} placeholder="搜索名称或网址" />{search && <button onClick={() => setSearch("")}><X size={13} /></button>}</label></header>
      <div className="command-bar">
        <button onClick={() => setNewOpen(true)}><Plus size={14} />新建</button><span className="separator" />
        <button disabled={!selected.size} onClick={() => void bulk("resume")}><Play size={14} />开始</button>
        <button disabled={!selected.size} onClick={() => void bulk("pause")}><Pause size={14} />暂停</button>
        <button disabled={!selected.size} onClick={() => void removeSelected(false)}><Trash2 size={14} />删除</button>
        <span className="separator" /><button disabled={!selectedOne || selectedOne.status !== "completed"} onClick={() => selectedOne && void api.openFile(selectedOne.id)}><ExternalLink size={14} />打开</button>
        <button disabled={!selectedOne} onClick={() => selectedOne && void api.openFolder(selectedOne.id)}><FolderOpen size={14} />文件夹</button>
        <span className="command-spacer" /><button onClick={() => setShowDetails((value) => !value)}>{showDetails ? <PanelRightClose size={15} /> : <PanelRightOpen size={15} />}</button>
        <button onClick={() => void refresh()}><RefreshCw size={14} /></button>
      </div>
      {fatal && <div className="error-banner"><Unplug size={16} /><span>无法连接下载内核：{fatal}</span><button onClick={() => void refresh()}>重试</button></div>}
      <section className={showDetails && selectedOne ? "content-grid details-on" : "content-grid"}>
        <div className="task-list-panel"><div className="task-grid">
          <div className="table-header"><label><input type="checkbox" aria-label="全选" checked={visible.length > 0 && visible.every((task) => selected.has(task.id))} onChange={() => setSelected(visible.every((task) => selected.has(task.id)) ? new Set() : new Set(visible.map((task) => task.id)))} /></label>{[["file_name","名称"],["total_bytes","大小"],["status","状态"],["downloaded_bytes","进度"],["speed","速度"],["created_at","添加时间"]].map(([key,label]) => <span key={key} onClick={() => setSort((current) => ({ key: key as keyof DownloadTask, desc: current.key === key ? !current.desc : key === "created_at" }))}>{label}</span>)}<span /></div>
          <div className="task-rows">{loading ? <div className="center-state"><LoaderCircle className="spin" /></div> : visible.length === 0 ? <EmptyState filter={filter} onAdd={() => setNewOpen(true)} /> : visible.map((task) => <TaskRow key={task.id} task={task} selected={selected.has(task.id)} onSelect={() => setSelected((current) => { const next = new Set(current); next.has(task.id) ? next.delete(task.id) : next.add(task.id); return next; })} onOpen={() => task.status === "completed" && void api.openFile(task.id)} onContext={(event) => { event.preventDefault(); setContext({ x: event.clientX, y: event.clientY, id: task.id }); if (!selected.has(task.id)) setSelected(new Set([task.id])); }} />)}</div>
        </div></div>
        {showDetails && selectedOne && <Details task={selectedOne} onClose={() => setSelected(new Set())} notify={notify} />}
      </section>
      <footer className="status-bar"><span className={isDesktop() ? "online" : "offline"}>{isDesktop() ? "下载服务已连接" : "仅界面预览"}</span><span>{active.length} 个活动任务</span><span>↓ {formatBytes(totalSpeed)}/s</span><span className="status-spacer" /><span>并发 {settings.concurrent_downloads}</span><button onClick={() => setSettingsOpen(true)}><Gauge size={12} /> {settings.speed_limit_kbps ? `${settings.speed_limit_kbps} KB/s` : "不限速"}</button></footer>
    </main>
    {newOpen && <NewTaskDialog settings={settings} onClose={() => setNewOpen(false)} onCreated={(count) => { setNewOpen(false); notify(`已添加 ${count} 个任务`); }} />}
    {context && <ContextMenu x={context.x} y={context.y} task={tasks.find((task) => task.id === context.id)!} close={() => setContext(undefined)} notify={notify} />}
    {toast && <div className="toast"><span>{toast.kind === "ok" ? <Check size={14} /> : <AlertCircle size={14} />}</span>{toast.text}</div>}
  </div>;
}

function TaskRow({ task, selected, onSelect, onOpen, onContext }: { task: DownloadTask; selected: boolean; onSelect: () => void; onOpen: () => void; onContext: (event: MouseEvent) => void }) {
  const progress = task.total_bytes ? Math.min(100, task.downloaded_bytes / task.total_bytes * 100) : 0;
  return <div className={selected ? "task-row selected" : "task-row"} onDoubleClick={onOpen} onContextMenu={onContext}>
    <label><input type="checkbox" aria-label={`选择 ${task.file_name}`} checked={selected} onChange={onSelect} /></label>
    <div className="name-cell" onClick={onSelect}><FileIcon category={task.category} /><div><strong title={task.file_name}>{task.file_name}</strong><small title={task.url}>{hostOf(task.url)}</small></div></div>
    <span>{task.total_bytes ? formatBytes(task.total_bytes) : "—"}</span><span className={`task-status ${task.status}`}>{task.status === "downloading" && task.eta_seconds ? `${statusText[task.status]} · ${formatDuration(task.eta_seconds)}` : statusText[task.status]}</span>
    <div className="progress-cell"><div><i style={{ width: `${progress}%` }} /></div><span>{task.status === "completed" ? "100%" : `${progress.toFixed(0)}%`}</span></div>
    <span>{task.status === "downloading" ? `${formatBytes(task.speed)}/s` : "—"}</span><span>{formatDate(task.created_at)}</span><button className="row-menu"><MoreHorizontal size={15} /></button>
  </div>;
}
function FileIcon({ category }: { category: string }) { const Icon = category === "video" ? Film : category === "audio" ? FileAudio : category === "images" ? FileImage : category === "archives" ? Archive : category === "apps" ? File : FileText; return <span className={`file-type ${category}`}><Icon size={16} /></span>; }
function EmptyState({ filter, onAdd }: { filter: FilterKey; onAdd: () => void }) { return <div className="empty-state"><Download size={27} /><h2>{filter === "all" ? "还没有下载任务" : "此分类中没有任务"}</h2><p>添加链接，或从 Chrome / Edge 扩展发送下载。</p><button onClick={onAdd}>新建任务</button></div>; }

function Details({ task, onClose, notify }: { task: DownloadTask; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void }) {
  const progress = task.total_bytes ? task.downloaded_bytes / task.total_bytes * 100 : 0;
  const action = async (value: string) => { try { await api.action(task.id, value); } catch (error) { notify(String(error), "error"); } };
  return <aside className="details-pane">
    <div className="details-file"><FileIcon category={task.category} /><div><h2>{task.file_name}</h2><p>{hostOf(task.url)}</p></div><button onClick={onClose}><X size={14} /></button></div>
    <div className="details-progress"><div><span>进度</span><strong>{progress.toFixed(1)}%</strong></div><div><i style={{ width: `${progress}%` }} /></div></div>
    {task.error && <div className="task-error">{task.error}</div>}
    <dl><div><dt>状态</dt><dd>{statusText[task.status]}</dd></div><div><dt>大小</dt><dd>{formatBytes(task.total_bytes)}</dd></div><div><dt>速度</dt><dd>{task.speed ? `${formatBytes(task.speed)}/s` : "—"}</dd></div><div><dt>剩余时间</dt><dd>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</dd></div><div><dt>保存位置</dt><dd>{task.destination}</dd></div><div><dt>来源</dt><dd>{task.source}</dd></div><div><dt>重试</dt><dd>{task.retry_count} / {task.max_retries}</dd></div>{task.checksum_sha256 && <div><dt>SHA-256</dt><dd title={task.checksum_sha256}>{task.checksum_sha256.slice(0, 16)}…</dd></div>}</dl>
    <div className="details-actions">{task.status === "downloading" ? <button onClick={() => void action("pause")}><Pause size={13} />暂停</button> : !["completed", "cancelled"].includes(task.status) && <button onClick={() => void action("resume")}><Play size={13} />继续</button>}<button onClick={() => void api.openFolder(task.id)}><FolderOpen size={13} />打开目录</button>{task.status === "completed" && <button onClick={async () => { try { const hash = await api.verify(task.id); notify(`校验完成：${hash.slice(0, 12)}…`); } catch (error) { notify(String(error), "error"); } }}><ShieldCheck size={13} />校验文件</button>}</div>
  </aside>;
}

function NewTaskDialog({ settings, onClose, onCreated }: { settings: AppSettings; onClose: () => void; onCreated: (count: number) => void }) {
  const [urls, setUrls] = useState(""); const [destination, setDestination] = useState(settings.download_dir);
  const [fileName, setFileName] = useState(""); const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false); const [error, setError] = useState<string>();
  const [schedule, setSchedule] = useState(""); const [policy, setPolicy] = useState<CollisionPolicy>(settings.default_collision_policy);
  const [referer, setReferer] = useState(""); const [cookie, setCookie] = useState(""); const [authorization, setAuthorization] = useState("");
  const [checksum, setChecksum] = useState(""); const [limit, setLimit] = useState(0);
  const [media, setMedia] = useState<MediaProbeResult>(); const [format, setFormat] = useState("");
  const lines = urls.split(/\r?\n/).map((value) => value.trim()).filter(Boolean);
  const probe = async () => { setBusy(true); setError(undefined); try { const result = await api.probeMedia(lines[0]); if (result.drm) throw new Error("检测到 DRM 保护，LumaGet 不处理此内容"); setMedia(result); const selected = result.formats.find((item) => item.has_video && item.has_audio) ?? result.formats.find((item) => item.has_video); setFormat(selected?.id ?? ""); if (!fileName) setFileName(`${safeDisplayName(result.title)}.mp4`); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  const submit = async () => {
    if (!lines.length) return; setBusy(true); setError(undefined);
    const headers: Record<string, string> = {}; if (referer) headers.Referer = referer; if (cookie) headers.Cookie = cookie; if (authorization) headers.Authorization = authorization;
    const template: Omit<NewTaskRequest, "url"> = { file_name: fileName || undefined, destination, headers, scheduled_at: schedule ? new Date(schedule).getTime() : undefined, priority: 0, expected_checksum: checksum || undefined, source: "desktop", per_task_speed_limit: limit * 1024, collision_policy: policy, media: media ? { extractor: media.extractor, format_id: format, format_label: media.formats.find((item) => item.id === format)?.label, subtitles: [], thumbnail: media.thumbnail } : undefined };
    try { if (lines.length === 1) await api.add({ url: lines[0], ...template }); else await api.addBatch(lines, template); onCreated(lines.length); } catch (reason) { setError(String(reason)); setBusy(false); }
  };
  return <Modal title="新建下载任务" onClose={onClose} wide><div className="new-task-form">
    <label className="form-field"><span>下载链接（每行一个）</span><textarea autoFocus value={urls} onChange={(e) => { setUrls(e.target.value); setMedia(undefined); }} placeholder="https://example.com/file.zip" /></label>
    <div className="form-row"><label className="form-field grow"><span>保存位置</span><input value={destination} onChange={(e) => setDestination(e.target.value)} /></label><button className="input-button" onClick={async () => { const path = await pickPath({ directory: true, multiple: false, defaultPath: destination }); if (typeof path === "string") setDestination(path); }}><FolderOpen size={14} />浏览</button><label className="form-field"><span>重名处理</span><select value={policy} onChange={(e) => setPolicy(e.target.value as CollisionPolicy)}><option value="rename">自动重命名</option><option value="overwrite">覆盖</option><option value="skip">跳过</option></select></label></div>
    {lines.length === 1 && <div className="form-row"><label className="form-field grow"><span>文件名（可选）</span><input value={fileName} onChange={(e) => setFileName(e.target.value)} /></label><button className="input-button" disabled={busy} onClick={() => void probe()}><Video size={14} />分析媒体</button></div>}
    {media && <div className="media-result"><strong>{media.title}</strong><select value={format} onChange={(e) => setFormat(e.target.value)}>{media.formats.filter((item) => item.has_video || item.has_audio).map((item) => <option key={item.id} value={item.id}>{item.label}{item.file_size ? ` · ${formatBytes(item.file_size)}` : ""}</option>)}</select></div>}
    <button className={advanced ? "advanced-toggle up" : "advanced-toggle"} onClick={() => setAdvanced((value) => !value)}><ChevronDown size={13} />高级选项</button>
    {advanced && <div className="advanced-grid"><Field label="计划开始"><input type="datetime-local" value={schedule} onChange={(e) => setSchedule(e.target.value)} /></Field><Field label="单任务限速（KB/s）"><input type="number" min="0" value={limit} onChange={(e) => setLimit(+e.target.value)} /></Field><Field label="Referer"><input value={referer} onChange={(e) => setReferer(e.target.value)} /></Field><Field label="Cookie"><input value={cookie} onChange={(e) => setCookie(e.target.value)} /></Field><Field label="Authorization"><input value={authorization} onChange={(e) => setAuthorization(e.target.value)} placeholder="Bearer … 或 Basic …" /></Field><Field label="预期 SHA-256"><input value={checksum} onChange={(e) => setChecksum(e.target.value)} /></Field></div>}
    {error && <div className="inline-error">{error}</div>}<div className="dialog-actions"><button onClick={onClose}>取消</button><button className="primary" disabled={busy || !lines.length} onClick={() => void submit()}>{busy ? "处理中…" : "开始下载"}</button></div>
  </div></Modal>;
}

type SettingsSection = "general" | "download" | "network" | "browser" | "media" | "appearance" | "advanced";
function SettingsPage({ value, onChange, onClose, notify }: { value: AppSettings; onChange: (value: AppSettings) => void; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void }) {
  const [draft, setDraft] = useState(value); const [section, setSection] = useState<SettingsSection>("general");
  const [pair, setPair] = useState<PairingInfo>(); const [tools, setTools] = useState<ToolStatus[]>([]);
  const set = <K extends keyof AppSettings>(key: K, val: AppSettings[K]) => setDraft((item) => ({ ...item, [key]: val }));
  useEffect(() => { if (section === "browser") void api.pairing().then(setPair); if (section === "media") void api.toolStatus().then(setTools); }, [section]);
  const save = async () => { try { await api.saveSettings(draft); onChange(draft); notify("设置已保存"); onClose(); } catch (error) { notify(String(error), "error"); } };
  const items: Array<[SettingsSection, string, typeof Settings]> = [["general","常规",Settings],["download","下载",Download],["network","网络",Network],["browser","浏览器",Globe2],["media","媒体",Video],["appearance","外观",SlidersHorizontal],["advanced","高级",Info]];
  return <div className="settings-page"><aside className="nav-pane"><div className="brand">设置</div>{items.map(([key,label,Icon]) => <button key={key} className={section === key ? "nav-item active" : "nav-item"} onClick={() => setSection(key)}><Icon size={15} /><span>{label}</span></button>)}<button className="nav-settings" onClick={onClose}><X size={15} /><span>返回任务</span></button></aside><main className="settings-body"><div className="settings-title"><h1>{items.find(([key]) => key === section)?.[1]}</h1></div><div className="settings-content">
    {section === "general" && <SettingsGroup title="应用行为"><Toggle label="启动时最小化" checked={draft.start_minimized} onChange={(v) => set("start_minimized", v)} /><Toggle label="最小化到托盘" checked={draft.minimize_to_tray} onChange={(v) => set("minimize_to_tray", v)} /><Toggle label="关闭时驻留托盘" checked={draft.close_to_tray} onChange={(v) => set("close_to_tray", v)} /><Toggle label="下载完成通知" checked={draft.notifications} onChange={(v) => set("notifications", v)} /><Toggle label="监视剪贴板链接" checked={draft.clipboard_monitor} onChange={(v) => set("clipboard_monitor", v)} /></SettingsGroup>}
    {section === "download" && <><SettingsGroup title="保存与性能"><SettingRow label="默认下载目录"><input value={draft.download_dir} onChange={(e) => set("download_dir", e.target.value)} /></SettingRow><SettingRow label="文件重名"><select value={draft.default_collision_policy} onChange={(e) => set("default_collision_policy", e.target.value as CollisionPolicy)}><option value="rename">自动重命名</option><option value="overwrite">覆盖</option><option value="skip">跳过</option></select></SettingRow><SettingRow label="同时下载任务"><input type="number" min="1" max="16" value={draft.concurrent_downloads} onChange={(e) => set("concurrent_downloads", +e.target.value)} /></SettingRow><SettingRow label="每任务连接数"><select value={draft.connections_per_download} onChange={(e) => set("connections_per_download", +e.target.value)}>{[1,2,4,8,16].map((n) => <option key={n}>{n}</option>)}</select></SettingRow><SettingRow label="全局限速（KB/s）"><input type="number" min="0" value={draft.speed_limit_kbps} onChange={(e) => set("speed_limit_kbps", +e.target.value)} /></SettingRow><Toggle label="完成后计算 SHA-256" checked={draft.verify_after_download} onChange={(v) => set("verify_after_download", v)} /></SettingsGroup></>}
    {section === "network" && <SettingsGroup title="代理与重试"><SettingRow label="代理模式"><select value={draft.proxy_mode} onChange={(e) => set("proxy_mode", e.target.value as AppSettings["proxy_mode"])}><option value="system">跟随系统</option><option value="none">不使用代理</option><option value="manual">手动代理</option></select></SettingRow>{draft.proxy_mode === "manual" && <><SettingRow label="代理地址"><input value={draft.proxy_url} onChange={(e) => set("proxy_url", e.target.value)} /></SettingRow><SettingRow label="用户名"><input value={draft.proxy_username} onChange={(e) => set("proxy_username", e.target.value)} /></SettingRow><SettingRow label="密码"><input type="password" value={draft.proxy_password} onChange={(e) => set("proxy_password", e.target.value)} /></SettingRow></>}<SettingRow label="最大重试次数"><input type="number" min="0" max="10" value={draft.max_retries} onChange={(e) => set("max_retries", +e.target.value)} /></SettingRow></SettingsGroup>}
    {section === "browser" && <><SettingsGroup title="下载接管"><Toggle label="允许浏览器扩展接管下载" checked={draft.intercept_browser_downloads} onChange={(v) => set("intercept_browser_downloads", v)} /><SettingRow label="最小文件大小（MB）"><input type="number" min="0" value={draft.min_file_size_mb} onChange={(e) => set("min_file_size_mb", +e.target.value)} /></SettingRow></SettingsGroup><SettingsGroup title="安全配对">{pair ? <div className="pair-card"><p>在扩展中输入一次性配对码（10 分钟有效）</p><code>{pair.code}</code>{pair.paired_extension && <p>已配对：{pair.paired_extension.slice(0, 16)}…</p>}<div className="maintenance"><button onClick={() => void api.rotatePairing().then(setPair)}>更换配对码</button>{pair.paired_extension && <button onClick={() => void api.revokePairing().then(() => api.pairing().then(setPair))}>撤销配对</button>}</div></div> : <LoaderCircle className="spin" />}</SettingsGroup></>}
    {section === "media" && <SettingsGroup title="媒体工具"><p className="settings-note">使用 yt-dlp 分析媒体，FFmpeg 合并音视频；DRM 内容明确拒绝处理。</p>{tools.map((tool) => <div className="tool-row" key={tool.name}><span>{tool.available ? <Check size={13} /> : <AlertCircle size={13} />} {tool.name}</span><small>{tool.available ? tool.version || tool.path : "尚未安装"}</small></div>)}<Toggle label="自动检查媒体工具更新" checked={draft.media_tool_auto_update} onChange={(v) => set("media_tool_auto_update", v)} /></SettingsGroup>}
    {section === "appearance" && <SettingsGroup title="主题"><SettingRow label="应用主题"><select value={draft.theme} onChange={(e) => set("theme", e.target.value as AppSettings["theme"])}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></select></SettingRow><p className="settings-note">使用 Windows 系统字体、中性色和单一强调色，不加载在线字体。</p></SettingsGroup>}
    {section === "advanced" && <SettingsGroup title="维护"><div className="maintenance"><button onClick={() => void api.clearHistory(false).then(() => notify("已清理取消的任务"))}>清理取消任务</button><button onClick={() => void api.clearHistory(true).then(() => notify("下载历史已清理"))}>清理完成和取消任务</button></div></SettingsGroup>}
  </div><div className="dialog-actions settings-actions"><button onClick={onClose}>取消</button><button className="primary" onClick={() => void save()}>保存设置</button></div></main></div>;
}

function SettingsGroup({ title, children }: { title: string; children: ReactNode }) { return <section className="settings-group"><h2>{title}</h2><div>{children}</div></section>; }
function SettingRow({ label, children }: { label: string; children: ReactNode }) { return <label className="setting-row"><div><strong>{label}</strong></div>{children}</label>; }
function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) { return <label className="setting-row"><div><strong>{label}</strong></div><input className="toggle" type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} /></label>; }
function Field({ label, children }: { label: string; children: ReactNode }) { return <label className="form-field"><span>{label}</span>{children}</label>; }
function ContextMenu({ x, y, task, close, notify }: { x: number; y: number; task: DownloadTask; close: () => void; notify: (text: string, kind?: "ok" | "error") => void }) { const action = async (value: string) => { try { await api.action(task.id, value); close(); } catch (error) { notify(String(error), "error"); } }; return <div className="context-menu" style={{ left: x, top: y }} onClick={(e) => e.stopPropagation()}>{task.status === "downloading" ? <button onClick={() => void action("pause")}><Pause size={13} />暂停</button> : !["completed","cancelled"].includes(task.status) && <button onClick={() => void action("resume")}><Play size={13} />开始 / 继续</button>}<button onClick={() => void api.openFolder(task.id).then(close)}><FolderOpen size={13} />打开文件夹</button><button onClick={() => void navigator.clipboard.writeText(task.url).then(() => { notify("链接已复制"); close(); })}><Copy size={13} />复制链接</button>{task.status === "completed" && <button onClick={() => void api.verify(task.id).then(() => { notify("文件校验完成"); close(); })}><ShieldCheck size={13} />校验 SHA-256</button>}<button className="danger" onClick={() => void api.remove(task.id, false).then(close)}><Trash2 size={13} />删除记录</button><button className="danger" onClick={() => void api.remove(task.id, true).then(close)}><Trash2 size={13} />删除记录和文件</button></div>; }
function Modal({ title, onClose, wide, children }: { title: string; onClose: () => void; wide?: boolean; children: ReactNode }) { return <div className="modal-layer" onMouseDown={onClose}><section className={wide ? "dialog wide" : "dialog"} onMouseDown={(e) => e.stopPropagation()}><div className="settings-title"><h2>{title}</h2><button onClick={onClose}><X size={15} /></button></div>{children}</section></div>; }
function formatBytes(value: number) { if (!value) return "0 B"; const units = ["B","KB","MB","GB","TB"]; const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1); return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`; }
function formatDuration(seconds: number) { if (seconds < 60) return `${seconds} 秒`; if (seconds < 3600) return `${Math.ceil(seconds / 60)} 分钟`; return `${Math.floor(seconds / 3600)} 小时 ${Math.ceil(seconds % 3600 / 60)} 分`; }
function formatDate(value: number) { return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(new Date(value)); }
function hostOf(url: string) { try { return new URL(url).host; } catch { return url; } }
function safeDisplayName(value: string) { return value.replace(/[<>:"/\\|?*]/g, "_").slice(0, 120); }
