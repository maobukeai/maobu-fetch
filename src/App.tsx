import { useEffect, useMemo, useState } from "react";
import {
  Activity, Archive, CheckCircle2, ChevronDown, CirclePause, Clock3, Download,
  File, FileAudio, FileImage, FileText, Film, Gauge, HardDrive, MoreHorizontal,
  Pause, Play, Plus, RefreshCw, Search, Settings, SlidersHorizontal, Trash2, X, Zap
} from "lucide-react";
import { api } from "./api";
import type { AppSettings, DownloadItem, FilterKey } from "./types";

const formatBytes = (value: number) => {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index > 1 ? 1 : 0)} ${units[index]}`;
};
const progress = (item: DownloadItem) => item.total_bytes ? Math.min(100, item.downloaded_bytes / item.total_bytes * 100) : 0;
const ext = (name: string) => name.split(".").pop()?.toLowerCase() || "";
const categoryOf = (name: string) => {
  const e = ext(name);
  if (["mp4", "mkv", "mov", "webm", "m3u8"].includes(e)) return "video";
  if (["mp3", "wav", "flac", "aac", "m4a"].includes(e)) return "audio";
  if (["jpg", "jpeg", "png", "gif", "webp", "svg"].includes(e)) return "images";
  if (["zip", "rar", "7z", "tar", "gz"].includes(e)) return "archives";
  if (["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt"].includes(e)) return "documents";
  if (["exe", "msi", "dmg", "pkg", "appimage"].includes(e)) return "apps";
  return "other";
};

const nav = [
  ["all", "全部下载", Download], ["downloading", "正在下载", Activity], ["queued", "等待中", Clock3],
  ["paused", "已暂停", CirclePause], ["completed", "已完成", CheckCircle2], ["failed", "未完成", RefreshCw]
] as const;
const cats = [["images", "图片", FileImage], ["video", "视频", Film], ["audio", "音频", FileAudio], ["documents", "文档", FileText], ["archives", "压缩包", Archive], ["apps", "应用", File]] as const;

function App() {
  const [items, setItems] = useState<DownloadItem[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [filter, setFilter] = useState<FilterKey>("all");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = async () => setItems(await api.list());
  useEffect(() => { void refresh(); void api.settings().then(setSettings); const timer = setInterval(refresh, 800); return () => clearInterval(timer); }, []);
  useEffect(() => {
    if (!settings) return;
    const dark = settings.theme === "dark" || (settings.theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [settings?.theme]);

  const shown = useMemo(() => items.filter(item => {
    const matchFilter = filter === "all" || item.status === filter || categoryOf(item.file_name) === filter;
    return matchFilter && `${item.file_name} ${item.url}`.toLowerCase().includes(query.toLowerCase());
  }), [items, filter, query]);
  const active = items.filter(i => i.status === "downloading");
  const totalSpeed = active.reduce((sum, i) => sum + i.speed, 0);
  const chosen = items.find(i => i.id === selected);

  const doAction = async (action: string, id: string) => { await api.action(action, id); await refresh(); };
  const submit = async () => {
    if (!url.trim()) return;
    setBusy(true);
    try { await api.add(url.trim()); setUrl(""); setAddOpen(false); await refresh(); } finally { setBusy(false); }
  };

  return <div className="app-shell">
    <div className="aurora a1" /><div className="aurora a2" />
    <aside className="sidebar glass">
      <div className="brand"><div className="brand-mark"><ArrowLogo /></div><div><b>LumaGet</b><span>Download beautifully.</span></div></div>
      <button className="primary-button" onClick={() => setAddOpen(true)}><Plus size={18} /> 新建下载</button>
      <nav>
        <p className="nav-title">下载</p>
        {nav.map(([key, label, Icon]) => <button key={key} className={filter === key ? "active" : ""} onClick={() => setFilter(key as FilterKey)}><Icon size={17} /><span>{label}</span><small>{key === "all" ? items.length : items.filter(i => i.status === key).length}</small></button>)}
        <p className="nav-title">分类</p>
        {cats.map(([key, label, Icon]) => <button key={key} className={filter === key ? "active" : ""} onClick={() => setFilter(key as FilterKey)}><Icon size={17} /><span>{label}</span></button>)}
      </nav>
      <div className="sidebar-foot">
        <button onClick={() => setSettingsOpen(true)}><Settings size={17} /><span>偏好设置</span></button>
        <div className="storage"><div><HardDrive size={15} /><span>下载空间</span><small>本地磁盘</small></div><div className="storage-bar"><i style={{ width: "36%" }} /></div></div>
      </div>
    </aside>

    <main>
      <header>
        <div><h1>{[...nav, ...cats].find(x => x[0] === filter)?.[1] || "全部下载"}</h1><p>{active.length ? `${active.length} 个任务正在传输` : "你的下载已整理妥当"}</p></div>
        <div className="header-actions">
          <label className="search"><Search size={17} /><input value={query} onChange={e => setQuery(e.target.value)} placeholder="搜索下载" /><kbd>⌘ K</kbd></label>
          <button className="icon-button"><SlidersHorizontal size={18} /></button>
        </div>
      </header>

      <section className="stats">
        <article className="stat-card glass"><div className="stat-icon violet"><Zap size={20} /></div><div><span>当前速度</span><strong>{formatBytes(totalSpeed)}<small>/s</small></strong></div><div className="spark"><i /><i /><i /><i /><i /><i /><i /></div></article>
        <article className="stat-card glass"><div className="stat-icon blue"><Activity size={20} /></div><div><span>活动任务</span><strong>{active.length}<small> 个</small></strong></div><span className="pill">并发 {settings?.concurrent_downloads || 3}</span></article>
        <article className="stat-card glass"><div className="stat-icon mint"><CheckCircle2 size={20} /></div><div><span>今日完成</span><strong>{items.filter(i => i.status === "completed").length}<small> 个</small></strong></div><span className="pill success">运行良好</span></article>
      </section>

      <section className="download-panel glass">
        <div className="panel-head"><div><b>下载任务</b><span>{shown.length} 个项目</span></div><div><button onClick={() => active.forEach(i => void doAction("pause", i.id))}><Pause size={15} /> 全部暂停</button><button><MoreHorizontal size={18} /></button></div></div>
        <div className="table-head"><span>名称</span><span>大小</span><span>进度</span><span>速度</span><span>状态</span><span /></div>
        <div className="download-list">
          {shown.map(item => <div className={`download-row ${selected === item.id ? "selected" : ""}`} key={item.id} onClick={() => setSelected(item.id)}>
            <div className="file-cell"><div className={`file-icon ${categoryOf(item.file_name)}`}><FileGlyph category={categoryOf(item.file_name)} /></div><div><b>{item.file_name}</b><span>{new URL(item.url).hostname}</span></div></div>
            <span>{formatBytes(item.total_bytes)}</span>
            <div className="progress-cell"><div><i style={{ width: `${progress(item)}%` }} /></div><span>{progress(item).toFixed(0)}%</span></div>
            <span>{item.status === "downloading" ? `${formatBytes(item.speed)}/s` : "—"}</span>
            <Status status={item.status} />
            <div className="row-actions">
              {item.status === "downloading" && <button onClick={e => { e.stopPropagation(); void doAction("pause", item.id); }}><Pause size={16} /></button>}
              {["paused", "queued", "failed"].includes(item.status) && <button onClick={e => { e.stopPropagation(); void doAction(item.status === "failed" ? "retry" : "resume", item.id); }}><Play size={16} /></button>}
              <button onClick={e => { e.stopPropagation(); setSelected(item.id); }}><MoreHorizontal size={18} /></button>
            </div>
          </div>)}
          {!shown.length && <div className="empty"><div><Download size={28} /></div><h3>这里还没有下载</h3><p>粘贴链接或从浏览器扩展发送任务</p><button className="primary-button" onClick={() => setAddOpen(true)}><Plus size={17} /> 新建下载</button></div>}
        </div>
      </section>
    </main>

    {chosen && <aside className="inspector glass"><button className="close" onClick={() => setSelected(null)}><X size={17} /></button><div className={`preview ${categoryOf(chosen.file_name)}`}><FileGlyph category={categoryOf(chosen.file_name)} /></div><h3>{chosen.file_name}</h3><p className="muted">{chosen.url}</p><div className="ring" style={{ "--p": `${progress(chosen) * 3.6}deg` } as React.CSSProperties}><div><strong>{progress(chosen).toFixed(0)}%</strong><span>{chosen.status === "downloading" ? "正在下载" : "任务详情"}</span></div></div><dl><div><dt>已下载</dt><dd>{formatBytes(chosen.downloaded_bytes)} / {formatBytes(chosen.total_bytes)}</dd></div><div><dt>保存至</dt><dd>{chosen.destination}</dd></div><div><dt>连接</dt><dd>{settings?.connections_per_download || 8} 个分段</dd></div></dl><div className="inspector-actions">{chosen.status === "downloading" ? <button onClick={() => void doAction("pause", chosen.id)}><Pause size={16} /> 暂停</button> : <button onClick={() => void doAction("resume", chosen.id)}><Play size={16} /> 继续</button>}<button className="danger" onClick={async () => { await api.remove(chosen.id, false); setSelected(null); await refresh(); }}><Trash2 size={16} /></button></div></aside>}

    {addOpen && <Modal title="新建下载" onClose={() => setAddOpen(false)}><label className="field"><span>下载链接</span><textarea autoFocus value={url} onChange={e => setUrl(e.target.value)} placeholder="https://example.com/file.zip" /></label><div className="hint"><Gauge size={16} /><span>链接会经过安全校验，并自动选择最佳连接数。</span></div><div className="modal-actions"><button onClick={() => setAddOpen(false)}>取消</button><button className="primary-button" disabled={busy || !url.trim()} onClick={() => void submit()}>{busy ? "正在添加…" : "开始下载"}</button></div></Modal>}
    {settingsOpen && settings && <Modal title="偏好设置" wide onClose={() => setSettingsOpen(false)}><SettingsForm value={settings} onChange={setSettings} /><div className="modal-actions"><button onClick={() => setSettingsOpen(false)}>取消</button><button className="primary-button" onClick={async () => { await api.saveSettings(settings); setSettingsOpen(false); }}>保存设置</button></div></Modal>}
  </div>;
}

function ArrowLogo() { return <svg viewBox="0 0 32 32"><path d="M16 4v16m0 0 7-7m-7 7-7-7"/><path d="M7 25h18"/></svg>; }
function FileGlyph({ category }: { category: string }) { const I = category === "video" ? Film : category === "audio" ? FileAudio : category === "images" ? FileImage : category === "archives" ? Archive : FileText; return <I size={21} />; }
function Status({ status }: { status: DownloadItem["status"] }) { const labels = { queued: "等待中", downloading: "下载中", paused: "已暂停", completed: "已完成", failed: "失败", cancelled: "已取消" }; return <span className={`status ${status}`}><i />{labels[status]}</span>; }
function Modal({ title, onClose, children, wide }: { title: string; onClose: () => void; children: React.ReactNode; wide?: boolean }) { return <div className="modal-backdrop" onMouseDown={onClose}><div className={`modal glass ${wide ? "wide" : ""}`} onMouseDown={e => e.stopPropagation()}><div className="modal-head"><h2>{title}</h2><button onClick={onClose}><X size={18} /></button></div>{children}</div></div>; }
function SettingsForm({ value, onChange }: { value: AppSettings; onChange: (s: AppSettings) => void }) { const set = <K extends keyof AppSettings>(key: K, val: AppSettings[K]) => onChange({ ...value, [key]: val }); return <div className="settings-grid"><label className="field"><span>默认下载目录</span><input value={value.download_dir} onChange={e => set("download_dir", e.target.value)} /></label><label className="field"><span>同时下载任务</span><select value={value.concurrent_downloads} onChange={e => set("concurrent_downloads", +e.target.value)}>{[1,2,3,4,5,6].map(x => <option key={x}>{x}</option>)}</select></label><label className="field"><span>每任务连接数</span><select value={value.connections_per_download} onChange={e => set("connections_per_download", +e.target.value)}>{[1,2,4,8,16].map(x => <option key={x}>{x}</option>)}</select></label><label className="field"><span>速度限制（KB/s，0 为不限）</span><input type="number" min="0" value={value.speed_limit_kbps} onChange={e => set("speed_limit_kbps", +e.target.value)} /></label><label className="field"><span>外观</span><select value={value.theme} onChange={e => set("theme", e.target.value as AppSettings["theme"])}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></select></label><label className="toggle"><span><b>接管浏览器下载</b><small>通过扩展自动发送符合条件的文件</small></span><input type="checkbox" checked={value.intercept_browser_downloads} onChange={e => set("intercept_browser_downloads", e.target.checked)} /></label></div>; }

export default App;
