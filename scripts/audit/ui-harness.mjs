// 仅由审查用 Vite 中间件提供，不进入生产应用入口。
export const uiHarness = `<!doctype html><html><head><meta charset="UTF-8"><link rel="icon" href="data:,"></head>
<body><div id="root"></div><script type="module">
import React from 'react';
import { createRoot } from 'react-dom/client';
import App from '/src/App.tsx';
import { Select } from '/src/components/Select.tsx';
import { CompletionToast } from '/src/components/CompletionToast.tsx';
import { api } from '/src/api.ts';
import '/src/styles.css';
const params = new URLSearchParams(location.search);
const calls = [];
const listeners = new Set();
let taskListener;
let settingListener;
const count = Number(params.get('count') || 0);
const tasks = Array.from({length: count}, (_, i) => ({
  id: 'audit-' + i, url: 'https://example.invalid/' + i,
  file_name: i === 0 ? '长文件名与特殊字符—'.repeat(18) + '.mp4' : '审查测试-' + i + '.zip',
  destination: 'C:\\AuditFixtures\\file-' + i + '.zip', total_bytes: 1048576,
  downloaded_bytes: i % 5 === 0 ? 1048576 : 524288, speed: 0, status: i % 5 === 0 ? 'completed' : 'paused',
  created_at: Date.now(), completed_at: Date.now(), category: 'archives',
  queue_position: i, priority: 0, retry_count: 0, max_retries: 3, source: 'desktop',
  headers: {}, per_task_speed_limit: 0, collision_policy: 'rename',
  completion_action: 'none', connection_count: 1, active_connections: 0, segments: [], task_kind: 'http',
}));
if (params.get('mode') === 'recording') {
  tasks.splice(0, tasks.length, ...Array.from({length:4}, (_, i) => ({
    id:'audit-' + i, url:'https://example.invalid/recording-' + i,
    file_name:['直播录制测试','普通点播测试','已暂停的直播','普通文件下载'][i] + '.mp4',
    destination:'C:/AuditFixtures', total_bytes:0, downloaded_bytes:65536,
    speed:i === 2 ? 0 : 8192, status:i === 2 ? 'paused' : 'downloading',
    created_at:Date.now() - i, category:'video', queue_position:i, priority:0,
    retry_count:0, max_retries:3, source:'desktop', headers:{}, per_task_speed_limit:0,
    collision_policy:'rename', completion_action:'none', connection_count:1,
    active_connections:i === 2 ? 0 : 1, segments:[], task_kind:'http',
    ...(i < 3 ? {media:{format_id:'fixture',is_live:i !== 1}} : {}),
  })));
}
api.list = async () => tasks;
api.subscribe = async (fn) => { taskListener = fn; return []; };
api.subscribeSettings = async (fn) => { settingListener = fn; return () => {}; };
api.subscribeTaskNotification = async (fn) => { listeners.add(fn); return () => listeners.delete(fn); };
api.openFile = async (id) => { calls.push(['openFile', id]); if (window.audit.failOpen) throw new Error('测试：文件不存在'); };
api.openFolder = async (id) => { calls.push(['openFolder', id]); };
api.action = async (id, action) => {
  calls.push(['action',id,action]);
  if (window.audit.holdAction === action) await new Promise(resolve => { window.audit.releaseAction=resolve; });
  if (window.audit.failAction === action) throw new Error('测试：录制尚未保存，原始数据已保留');
};
api.saveSettings = async (settings) => { window.audit.saved = settings; };
api.getWaitReason = async () => ({ reason: 'unknown' });
api.getSpeedHistory = async () => [];
api.diagnose = async () => null;
window.audit = { calls, tasks, failOpen: false,
  emit: (id = 'audit-0') => listeners.forEach(fn => fn({ task_id: id, kind: 'completed', title: '下载完成：' + tasks[0]?.file_name, body: '已保存到测试目录' })),
  updateTask: (task) => taskListener?.({task}),
  updateSettings: (settings) => settingListener?.(settings),
};
function NoticeHarness() {
  const [notices, setNotices] = React.useState([]);
  const dismiss = React.useCallback(id => setNotices(items => items.filter(x => x.task_id !== id)), []);
  window.audit.emit = (id = 'audit-0') => setNotices(items => [...items, {task_id:id, kind:'completed', title:'下载完成：' + '很长的文件名'.repeat(22) + '.mp4', body:'已保存到测试目录'}]);
  return React.createElement('div', {className:'toast-stack'}, React.createElement(CompletionToast, {
    notices, durationSeconds:Number(params.get('duration') ?? 30), onDismiss:dismiss,
    onView:id => calls.push(['view', id]), onError:message => calls.push(['error', message]),
  }));
}
function SelectHarness() {
  const [value,setValue] = React.useState('first');
  return React.createElement('div', null,
    React.createElement(Select, {ariaLabel:'键盘测试',value,onChange:next => {setValue(next); window.audit.selected=next;},options:[
      {value:'first',label:'第一个'}, {value:'blocked',label:'禁用选项',disabled:true}, {value:'third',label:'第三个'},
    ]}), React.createElement('button',{id:'after-select'},'下一个控件'));
}
createRoot(document.getElementById('root')).render(React.createElement(params.get('mode') === 'notice' ? NoticeHarness : params.get('mode') === 'select' ? SelectHarness : App));
</script></body></html>`;
