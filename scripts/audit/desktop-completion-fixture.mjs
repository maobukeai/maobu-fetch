// Browser-only stand-in for the native queue. React owns the actual countdown;
// this fixture only acknowledges exact task/notice IDs and simulates IPC results.
export const desktopCompletionFixture = `<!doctype html><html><head><meta charset="UTF-8"><link rel="icon" href="data:,"></head>
<body><div id="root"></div><script type="module">
import React from 'react';
import { createRoot } from 'react-dom/client';
import { DesktopCompletionView } from '/src/components/desktop-completion/DesktopCompletionView.tsx';
import { desktopCompletionApi } from '/src/components/desktop-completion/desktop-completion-api.ts';
import '/src/styles.css';
const params = new URLSearchParams(location.search);
const listeners = new Set();
const calls = [];
let revision = 1;
let serial = 0;
const notice = (id = 'desktop-' + serial) => ({task_id:id,notice_id:++serial,
  title:params.get('long') === '1' ? '下载完成：' + '很长的中文文件名&<>'.repeat(20) + '.mp4' : '下载完成：示例文件-' + serial + '.zip',
  body:params.get('long') === '1' ? '已保存到 C:/Downloads/' + '很长的目录/'.repeat(15) : '已保存到 C:/Downloads'});
let queue = Array.from({length:Number(params.get('count') ?? 1)}, () => notice());
const copy = () => structuredClone({revision,notice:queue[0] ?? null,pending_count:Math.max(0,queue.length - 1),
  duration_seconds:Number(params.get('duration') ?? 30),color_scheme:'system',accent_color:'blue',language:params.get('lang') ?? 'zh-CN'});
const publish = () => { revision++; listeners.forEach(fn => fn(copy())); return copy(); };
const root = document.getElementById('root');
Object.assign(root.style,{position:'fixed',right:'12px',bottom:'12px',width:'352px',height:'auto'});
const audit = window.audit = {calls,get snapshot() {return copy();},get listenerCount() {return listeners.size;},
  append:id => {queue.push(notice(id));return publish();},
  replace:id => {queue = [notice(id)];return publish();},
  emit:snapshot => listeners.forEach(fn => fn(snapshot)),
  failAction:null,holdAction:null,presentAttempts:0,presentedId:null,
};
desktopCompletionApi.snapshot = async () => {
  audit.initialCalled = true;
  const snapshot = copy();
  if (params.get('race') === '1') await new Promise(resolve => setTimeout(resolve, 180));
  return snapshot;
};
desktopCompletionApi.subscribe = async fn => {listeners.add(fn);return () => listeners.delete(fn);};
desktopCompletionApi.present = async ({task_id:taskId,notice_id:noticeId}, height) => {
  if (queue[0]?.task_id !== taskId || queue[0]?.notice_id !== noticeId) throw new Error('测试：提醒已切换');
  calls.push(['present',taskId,noticeId,height]);
  audit.presentAttempts++;
  if (audit.presentAttempts <= Number(params.get('presentFail') ?? 0)) throw new Error('测试：原生窗口尚未就绪');
  audit.presentedId = noticeId;
  return copy();
};
desktopCompletionApi.act = async ({task_id:taskId,notice_id:noticeId}, action) => {
  calls.push(['act',taskId,noticeId,action]);
  if (queue[0]?.task_id !== taskId || queue[0]?.notice_id !== noticeId) throw new Error('测试：提醒已切换');
  if (audit.holdAction === action) await new Promise(resolve => {audit.releaseAction = resolve;});
  if (audit.failAction === action) throw new Error(audit.actionError ?? '测试：文件已移动，请检查保存位置后重试');
  queue.shift();
  return publish();
};
createRoot(root).render(React.createElement(React.StrictMode,null,React.createElement(DesktopCompletionView)));
</script></body></html>`;
