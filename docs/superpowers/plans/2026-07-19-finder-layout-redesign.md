# Finder Layout Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize the Maobu Fetch application layout into a classic Finder style: merge header bars, move the details panel to the right side, and integrate status information into the left sidebar footer.

**Architecture:** 
- Workspace grid layout will be reduced from 4 rows to 2 rows (Unified Toolbar + Main content).
- Content split will be refactored from vertical (top table + bottom details) to horizontal (left table + right details drawer).
- Status parameters will be rendered inside the sidebar component under a unified `.nav-footer` module.

**Tech Stack:** React, TypeScript, CSS Grid/Flexbox, Lucide React

## Global Constraints

- Do not perform any Git commit or staging operations. All work must remain uncommitted.
- The sidebar width must be kept at 184px.
- Follow existing dark/light theme color tokens.

---

### Task 1: Refactor Sidebar Footer Layout and Integrate Status Metrics

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/styles.css`

**Interfaces:**
- Consumes: `totalSpeed`, `active`, `settings` from `src/App.tsx`
- Produces: Sidebar bottom `.nav-footer` containing settings and connection status metrics.

- [ ] **Step 1: Modify Sidebar JSX inside `src/App.tsx`**
  Modify `<aside className="nav-pane">` to wrap the settings button and the new status information block inside a `.nav-footer` block at the bottom of the sidebar.
  Replace:
  ```tsx
  <button className="nav-settings" onClick={() => setSettingsOpen(true)}><Settings size={15} /><span>设置</span></button>
  ```
  With:
  ```tsx
  <div className="nav-footer">
    <button className="nav-settings" onClick={() => setSettingsOpen(true)}><Settings size={15} /><span>设置</span></button>
    <div className="nav-status" onClick={() => setSettingsOpen(true)}>
      <i className={isDesktop() ? "status-dot online" : "status-dot offline"} />
      <span>↓ {formatBytes(totalSpeed)}/s · {active.length} 活动</span>
    </div>
  </div>
  ```

- [ ] **Step 2: Modify Sidebar CSS inside `src/styles.css`**
  Modify `.nav-settings` to remove its `margin-top: auto`. Add styling for `.nav-footer` and `.nav-status`.
  Replace:
  ```css
  .nav-settings { margin-top: auto; }
  ```
  With:
  ```css
  .nav-footer {
    margin-top: auto;
    padding-top: 10px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .nav-settings {
    margin-top: 0;
  }
  .nav-status {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px;
    font-size: 10px;
    color: var(--subtle);
    cursor: pointer;
    user-select: none;
    font-variant-numeric: tabular-nums;
  }
  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #a0a0a0;
  }
  .status-dot.online {
    background: var(--success);
    box-shadow: 0 0 4px var(--success);
  }
  .status-dot.offline {
    background: var(--danger);
  }
  ```

- [ ] **Step 3: Run Typescript Check**
  Run: `npx pnpm run check`
  Expected: Command finishes successfully with no TS errors.

---

### Task 2: Merge Workspace Titlebar and Command Bar into Unified Header

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/styles.css`

**Interfaces:**
- Consumes: Action callback buttons and handlers in `App` component.
- Produces: Merged `.titlebar` layout containing title, toolbar actions, search box, and details toggle.

- [ ] **Step 1: Modify Workspace JSX structure in `src/App.tsx`**
  Unify the header JSX structure by combining the titlebar and command-bar elements into a single `<header className="titlebar">`. Remove the original `.command-bar` element and the separate `footer` element (since its status items were moved to the sidebar).
  Replace the workspace JSX (lines 206-227):
  ```tsx
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
    ...
    <footer className="status-bar"><span className={isDesktop() ? "online" : "offline"}>{isDesktop() ? "下载服务已连接" : "仅界面预览"}</span><span>{active.length} 个活动任务</span><span>↓ {formatBytes(totalSpeed)}/s</span><span className="status-spacer" /><span>并发 {settings.concurrent_downloads}</span><button onClick={() => setSettingsOpen(true)}><Gauge size={12} /> {settings.speed_limit_kbps ? `${settings.speed_limit_kbps} KB/s` : "不限速"}</button></footer>
  </main>
  ```
  With:
  ```tsx
  <main className="workspace">
    <header className="titlebar">
      <h1>{sectionTitle}</h1>
      <div className="toolbar-actions">
        <button onClick={() => setNewOpen(true)} title="新建"><Plus size={14} /></button>
        <button disabled={!selected.size} onClick={() => void bulk("resume")} title="开始"><Play size={14} /></button>
        <button disabled={!selected.size} onClick={() => void bulk("pause")} title="暂停"><Pause size={14} /></button>
        <button disabled={!selected.size} onClick={() => void removeSelected(false)} title="删除"><Trash2 size={14} /></button>
        <span className="separator" />
        <button disabled={!selectedOne || selectedOne.status !== "completed"} onClick={() => selectedOne && void api.openFile(selectedOne.id)} title="打开"><ExternalLink size={14} /></button>
        <button disabled={!selectedOne} onClick={() => selectedOne && void api.openFolder(selectedOne.id)} title="文件夹"><FolderOpen size={14} /></button>
        <button onClick={() => void refresh()} title="刷新"><RefreshCw size={14} /></button>
      </div>
      <label className="search-box"><Search size={14} /><input aria-label="搜索任务" value={search} onChange={(e) => setSearch(e.target.value)} placeholder="搜索名称或网址" />{search && <button onClick={() => setSearch("")}><X size={13} /></button>}</label>
      <button className="details-toggle" onClick={() => setShowDetails((value) => !value)} title="详情面板">{showDetails ? <PanelRightClose size={15} /> : <PanelRightOpen size={15} />}</button>
    </header>
    {fatal && <div className="error-banner"><Unplug size={16} /><span>无法连接下载内核：{fatal}</span><button onClick={() => void refresh()}>重试</button></div>}
    ...
  </main>
  ```

- [ ] **Step 2: Modify Workspace Grid CSS in `src/styles.css`**
  Modify `.workspace` grid rows to remove the command bar row (40px) and status bar row (24px). Remove `.status-bar` styles and add styles for `.toolbar-actions` and `.details-toggle`.
  Replace:
  ```css
  .workspace { display: grid; grid-template-rows: 78px 40px minmax(0, 1fr) 24px; min-width: 0; min-height: 0; background: var(--surface-solid); }
  .titlebar { display: flex; align-items: center; gap: 18px; padding: 32px 18px 8px; border-bottom: none; background: var(--surface-solid); }
  .titlebar h1 { margin: 0; min-width: 150px; font-size: 18px; font-weight: 600; letter-spacing: -.2px; }
  .search-box { display: flex; align-items: center; gap: 8px; width: min(420px, 42vw); height: 30px; margin-left: auto; padding: 0 9px; border: 1px solid var(--border); border-radius: 6px; background: var(--toggle-bg); color: var(--muted); transition: border-color 0.15s, box-shadow 0.15s; }
  ...
  .command-bar { display: flex; align-items: center; gap: 2px; padding: 0 12px 12px; border-bottom: 1px solid var(--border); background: var(--surface-solid); }
  .command-bar button, .details-actions button, .maintenance button, .input-button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 28px; padding: 0 10px; border-radius: 6px; color: var(--text); background: transparent; font-size: 11px; }
  .command-bar button:hover:not(:disabled), .details-actions button:hover:not(:disabled), .maintenance button:hover, .input-button:hover { background: var(--surface-hover); }
  .command-bar .danger { color: var(--danger); }
  .separator { width: 1px; height: 16px; margin: 0 6px; background: var(--border); }
  .command-spacer { flex: 1; }
  ...
  .status-bar { display: flex; align-items: center; gap: 16px; padding: 0 12px; border-top: 1px solid var(--border); color: var(--muted); background: var(--surface-alt); font-size: 9px; }
  .status-spacer { flex: 1; }
  .online, .offline { display: inline-flex; align-items: center; gap: 5px; }
  .online::before, .offline::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--success); }
  .offline { color: var(--danger); }.offline::before { background: var(--danger); }
  ```
  With:
  ```css
  .workspace { display: grid; grid-template-rows: 56px minmax(0, 1fr); min-width: 0; min-height: 0; background: var(--surface-solid); }
  .titlebar { display: flex; align-items: center; gap: 14px; padding: 12px 16px; border-bottom: 1px solid var(--border); background: var(--surface-solid); z-index: 10; }
  .titlebar h1 { margin: 0; min-width: 80px; font-size: 15px; font-weight: 600; letter-spacing: -.2px; }
  .toolbar-actions { display: flex; align-items: center; gap: 2px; }
  .toolbar-actions button, .details-actions button, .maintenance button, .input-button { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; border-radius: 6px; color: var(--text); background: transparent; transition: background-color 0.15s; }
  .toolbar-actions button:hover:not(:disabled), .details-actions button:hover:not(:disabled), .maintenance button:hover, .input-button:hover { background: var(--surface-hover); }
  .toolbar-actions button:disabled { opacity: 0.4; }
  .separator { width: 1px; height: 16px; margin: 0 4px; background: var(--border); }
  .search-box { display: flex; align-items: center; gap: 8px; width: min(300px, 30vw); height: 28px; margin-left: auto; padding: 0 9px; border: 1px solid var(--border); border-radius: 6px; background: var(--toggle-bg); color: var(--muted); transition: border-color 0.15s, box-shadow 0.15s; }
  .details-toggle { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; border-radius: 6px; color: var(--text); background: transparent; }
  .details-toggle:hover { background: var(--surface-hover); }
  ```

- [ ] **Step 3: Run Build Check**
  Run: `npx pnpm run build`
  Expected: Command completes successfully.

---

### Task 3: Refactor Details Pane to Vertical Right Sidebar Inspector

**Files:**
- Modify: `src/styles.css`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `Details` component properties in `src/App.tsx`.
- Produces: Collapsible Right-hand Side Panel (`.details-pane`) matching macOS Inspector.

- [ ] **Step 1: Modify Content Grid Layout in `src/styles.css`**
  Change `.content-grid` layout columns and refactor `.details-pane` to stand vertically.
  Replace:
  ```css
  .content-grid.details-on { grid-template-rows: minmax(180px, 1fr) 148px; }
  ...
  .details-pane { display: grid; grid-template-columns: minmax(175px, .7fr) minmax(320px, 1.4fr) minmax(270px, 1.2fr) auto; gap: 16px; align-items: start; min-height: 0; overflow: auto; padding: 11px 14px; border-top: 1px solid var(--border); background: var(--sidebar-bg); }
  .details-file { display: flex; align-items: center; gap: 8px; margin: 0; }
  .details-file > div:last-child { min-width: 0; }
  .details-file h2 { overflow-wrap: anywhere; margin: 0 0 4px; font-size: 13px; font-weight: 600; }
  .details-file p { overflow: hidden; margin: 0; color: var(--muted); font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
  .details-progress { display: none; }
  .details-progress > div:first-child { display: flex; justify-content: space-between; margin-bottom: 7px; font-size: 11px; }
  .details-progress > div:nth-child(2) { height: 5px; overflow: hidden; border-radius: 3px; background: var(--border); }
  .details-progress i { display: block; height: 100%; background: var(--accent); }
  .details-pane dl { display: grid; grid-template-columns: repeat(4, minmax(110px, 1fr)); gap: 5px 16px; margin: 0; }
  .details-pane dl div { display: flex; gap: 7px; min-width: 0; padding: 4px 0; border: 0; font-size: 9px; }
  .details-pane dl .wide { grid-column: span 2; }
  .details-pane dt { flex: 0 0 auto; color: var(--muted); }.details-pane dd { overflow: hidden; margin: 0; text-align: left; text-overflow: ellipsis; white-space: nowrap; }
  .segment-panel { min-width: 0; }
  .segment-title { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 7px; font-size: 9px; }
  .segment-title strong { color: var(--text); font-size: 10px; font-weight: 600; }
  .segment-title span { color: var(--muted); white-space: nowrap; }
  .segment-list { display: grid; grid-template-columns: repeat(2, minmax(105px, 1fr)); gap: 5px 10px; }
  .segment-item { display: grid; grid-template-columns: 18px minmax(48px, 1fr) 25px; align-items: center; gap: 5px; color: var(--muted); font-size: 8px; font-variant-numeric: tabular-nums; }
  .segment-item > div { height: 3px; overflow: hidden; border-radius: 2px; background: var(--border); }
  .segment-item i { display: block; height: 100%; background: var(--accent); }
  .segment-item em { color: var(--subtle); font-style: normal; text-align: right; }
  .task-error { margin: 12px 0; padding: 9px; border-radius: 4px; color: var(--danger); background: rgba(196,43,28,.08); font-size: 10px; }
  .details-actions { display: flex; flex-direction: column; gap: 3px; margin: 0; }
  ```
  With:
  ```css
  .content-grid.details-on { grid-template-columns: minmax(0, 1fr) 260px; grid-template-rows: 1fr; }
  ...
  .details-pane {
    display: flex;
    flex-direction: column;
    width: 260px;
    height: 100%;
    border-left: 1px solid var(--border);
    border-top: none;
    background: var(--sidebar-bg);
    overflow: hidden;
  }
  .details-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px;
    border-bottom: 1px solid var(--border);
  }
  .details-header h2 {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }
  .details-header button {
    width: 20px;
    height: 20px;
    border-radius: 4px;
    display: grid;
    place-items: center;
    color: var(--subtle);
  }
  .details-header button:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
  .details-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .details-scroll dl {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
  }
  .details-scroll dl div {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .details-scroll dt {
    font-size: 10px;
    color: var(--subtle);
  }
  .details-scroll dd {
    margin: 0;
    font-size: 11px;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .details-scroll .segment-panel {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .details-scroll .segment-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .details-scroll .segment-item {
    display: grid;
    grid-template-columns: 24px 1fr 28px;
    align-items: center;
    gap: 6px;
    font-size: 9px;
    color: var(--muted);
  }
  .details-scroll .segment-item > div {
    height: 4px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--border);
  }
  .details-scroll .segment-item i {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .details-scroll .segment-item em {
    font-style: normal;
    text-align: right;
  }
  .details-scroll .details-actions {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: auto;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .details-scroll .details-actions button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--control);
    font-size: 11px;
    transition: background-color 0.15s;
  }
  .details-scroll .details-actions button:hover {
    background: var(--surface-hover);
  }
  .details-scroll .details-actions .danger {
    color: var(--danger);
  }
  .task-error {
    padding: 8px;
    border-radius: 6px;
    color: var(--danger);
    background: rgba(255, 59, 48, 0.08);
    font-size: 10px;
    line-height: 1.4;
  }
  ```

- [ ] **Step 2: Modify Details Component layout in `src/App.tsx`**
  Modify the `Details` component definition to render with vertical scroll list elements (`.details-header`, `.details-scroll`, `.details-scroll dl`, etc.).
  Replace the `Details` component definition (lines 250-261):
  ```tsx
  function Details({ task, onClose, notify }: { task: DownloadTask; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void }) {
    const progress = task.total_bytes ? task.downloaded_bytes / task.total_bytes * 100 : 0;
    const action = async (value: string) => { try { await api.action(task.id, value); } catch (error) { notify(String(error), "error"); } };
    return <aside className="details-pane">
      <div className="details-file"><FileIcon category={task.category} /><div><h2>{task.file_name}</h2><p>{hostOf(task.url)}</p></div><button onClick={onClose}><X size={14} /></button></div>
      <div className="details-progress"><div><span>进度</span><strong>{progress.toFixed(1)}%</strong></div><div><i style={{ width: `${progress}%` }} /></div></div>
      {task.error && <div className="task-error">{task.error}</div>}
      <dl><div><dt>状态</dt><dd>{statusText[task.status]}</dd></div><div><dt>大小</dt><dd>{formatBytes(task.total_bytes)}</dd></div><div><dt>速度</dt><dd>{task.speed ? `${formatBytes(task.speed)}/s` : "—"}</dd></div><div><dt>剩余</dt><dd>{task.eta_seconds ? formatDuration(task.eta_seconds) : "—"}</dd></div><div className="wide"><dt>保存位置</dt><dd>{task.destination}</dd></div><div><dt>来源</dt><dd>{task.source}</dd></div><div><dt>重试</dt><dd>{task.retry_count} / {task.max_retries}</dd></div>{task.checksum_sha256 && <div className="wide"><dt>SHA-256</dt><dd title={task.checksum_sha256}>{task.checksum_sha256}</dd></div>}</dl>
      {task.segments.length > 0 && <div className="segment-panel"><div className="segment-title"><strong>分段连接</strong><span>{task.active_connections} 个活动 / {task.connection_count} 个配置</span></div><div className="segment-list">{task.segments.map((segment) => { const size = segment.end_byte - segment.start_byte + 1; const value = size ? Math.min(100, segment.downloaded_bytes / size * 100) : 0; return <div className="segment-item" key={segment.index}><span>#{segment.index + 1}</span><div><i style={{ width: `${value}%` }} /></div><em>{value.toFixed(0)}%</em></div>; })}</div></div>}
      <div className="details-actions">{task.status === "downloading" ? <button onClick={() => void action("pause")}><Pause size={13} />暂停</button> : !["completed", "cancelled"].includes(task.status) && <button onClick={() => void action("resume")}><Play size={13} />继续</button>}<button onClick={() => void api.openFolder(task.id)}><FolderOpen size={13} />打开目录</button>{task.status === "completed" && <button onClick={async () => { try { const hash = await api.verify(task.id); notify(`校验完成：${hash.slice(0, 12)}…`); } catch (error) { notify(String(error), "error"); } }}><ShieldCheck size={13} />校验文件</button>}</div>
    </aside>;
  }
  ```
  With:
  ```tsx
  function Details({ task, onClose, notify }: { task: DownloadTask; onClose: () => void; notify: (text: string, kind?: "ok" | "error") => void }) {
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
            <dt style={{ fontSize: "10px", color: "var(--subtle)", marginBottom: "4px" }}>分段连接 ({task.active_connections} 活动 / {task.connection_count} 分段)</dt>
            <div className="segment-list">
              {task.segments.map((segment) => {
                const size = segment.end_byte - segment.start_byte + 1;
                const value = size ? Math.min(100, (segment.downloaded_bytes / size) * 100) : 0;
                return (
                  <div className="segment-item" key={segment.index}>
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
  ```

- [ ] **Step 3: Run Verification check and build**
  Run: `npx pnpm run check`
  Expected: Success
  Run: `npx pnpm run build`
  Expected: Success
