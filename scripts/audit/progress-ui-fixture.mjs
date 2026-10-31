// Served only by the loopback audit server; never imported by a production entry.
export const progressUiFixture = `<!doctype html><html><head><meta charset="UTF-8"><link rel="icon" href="data:,"></head>
<body><div id="audit-backdrop" aria-hidden="true" style="position:fixed;inset:0;background:#d9dee6;pointer-events:none"></div>
<button id="audit-external-focus" style="position:fixed;top:8px;left:8px">外部测试控件</button><div id="root"></div><script type="module">
import React from 'react';
import { createRoot } from 'react-dom/client';
import { DownloadProgressView } from '/src/components/progress/DownloadProgressView.tsx';
import { progressApi } from '/src/components/progress/progress-api.ts';
import { ProgressWindowSettings } from '/src/components/settings/ProgressWindowSettings.tsx';
import '/src/styles.css';
const params = new URLSearchParams(location.search);
const count = Number(params.get('count') ?? 8);
const listeners = new Set();
const calls = [];
let snapshot = {
  revision: 1, total_count: count, total_speed_bps: count ? 131072 : 0,
  always_on_top: true, enabled: true, expanded: false, revealed: false, loading: false,
  summary_percent: params.get('known') === '1' && count
    ? count > 6 ? ((count - 1) * .5 + 9) / (count - 1 + 9) * 100 : 50 : null,
  theme: 'system', color_scheme: params.get('theme') ?? 'system', language: params.get('lang') ?? 'zh-CN', accent_color: 'blue', error: null,
  tasks: Array.from({length: Math.min(count, 6)}, (_, i) => ({
    id: 'progress-' + i,
    file_name: i === 0 ? '独立进度窗长文件名与中文字符'.repeat(20) + '.zip' : '审查任务-' + i + '.bin',
    status: ['downloading', 'downloading', 'verifying', 'queued', 'waiting-network', 'downloading'][i],
    total_bytes: i === 1 && params.get('known') !== '1' ? 0 : 1048576,
    downloaded_bytes: i === 1 && params.get('known') !== '1' ? 65536 : 524288,
    speed_bps: i === 0 ? 131072 : 0, fetching_metadata: false,
    eta_seconds: i === 0 ? 4 : null, can_pause: i !== 2, can_resume: false,
  })),
};
const copy = () => structuredClone(snapshot);
const rootElement = document.getElementById('root');
if (params.get('mode') !== 'settings') Object.assign(rootElement.style, {position:'fixed',right:'12px',bottom:'12px',width:'224px',height:'28px'});
let sizeTimer;
const resizeNativeFixture = () => {
  if (params.get('mode') === 'settings') return;
  clearTimeout(sizeTimer);
  sizeTimer = setTimeout(() => {
    const rows = Math.max(1, Math.min(snapshot.total_count, snapshot.expanded ? 6 : 3));
    rootElement.style.height = (snapshot.revealed ? 82 + rows * 36 : 28) + 'px';
  }, Number(params.get('resizeDelay') ?? 250));
};
const emit = () => { snapshot.revision++; resizeNativeFixture(); listeners.forEach(fn => fn(copy())); return copy(); };
window.audit = { calls, failSnapshot: params.get('fail') === '1', failPin: false, holdPin: false, holdReveal: false,
  emit: patch => { snapshot = {...snapshot, ...patch}; return emit(); },
  get snapshot() { return copy(); }, get listenerCount() { return listeners.size; },
};
progressApi.subscribe = async fn => { listeners.add(fn); return () => listeners.delete(fn); };
progressApi.snapshot = async () => {
  window.audit.initialCalled = true;
  const initial = copy();
  if (params.get('race') === '1') await new Promise(resolve => setTimeout(resolve, 180));
  if (window.audit.failSnapshot) throw new Error('synthetic private diagnostic');
  return initial;
};
progressApi.pinned = async pinned => {
  calls.push(['pin', pinned]);
  if (window.audit.holdPin) await new Promise(resolve => { window.audit.releasePin = resolve; });
  if (window.audit.failPin) throw new Error('Cookie: synthetic-private; token=synthetic-private');
  snapshot.always_on_top = pinned; return emit();
};
progressApi.expanded = async expanded => { calls.push(['expand', expanded]); snapshot.expanded = expanded; return emit(); };
progressApi.revealed = async revealed => {
  calls.push(['reveal', revealed]);
  if (window.audit.holdReveal) await new Promise(resolve => { window.audit.releaseReveal = resolve; });
  snapshot.revealed = revealed; return emit();
};
progressApi.action = async (id, action) => {
  calls.push(['action', id, action]); snapshot.tasks = snapshot.tasks.filter(task => task.id !== id);
  snapshot.total_count = Math.max(0, snapshot.total_count - 1); return emit();
};
progressApi.showMain = async id => { calls.push(['main', id ?? null]); return copy(); };
progressApi.dismiss = async () => { calls.push(['dismiss']); return copy(); };
progressApi.restore = async () => { calls.push(['restore']); return emit(); };
function SettingsHarness() {
  const savedEnabled = params.get('saved') === '1';
  const [draft, setDraft] = React.useState({progress_window_enabled: savedEnabled, progress_window_always_on_top: true});
  return React.createElement(ProgressWindowSettings, {value:draft,savedEnabled,
    onChange:patch=>{setDraft(current=>({...current,...patch}));calls.push(['draft',patch]);},
    notify:(...args)=>calls.push(['notify',...args])});
}
createRoot(document.getElementById('root')).render(React.createElement(React.StrictMode, null,
  React.createElement(params.get('mode') === 'settings' ? SettingsHarness : DownloadProgressView)));
</script></body></html>`;
