// Read-only IPC boundary probe. Does not start Tauri or access an application database.
import fs from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";
import assert from "node:assert/strict";

const root = new URL("../../", import.meta.url);
const apiSource = await fs.readFile(new URL("src/api.ts", root), "utf8");
const compiled = ts.transpileModule(apiSource, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
}).outputText;
const payloads = [];
const events = [];
const module = { exports: {} };
vm.runInNewContext(compiled, {
  module, exports: module.exports, window: { __TAURI_INTERNALS__: {} },
  require: (name) => {
    if (name === "@tauri-apps/api/core") return { invoke: async (command, args) => { payloads.push({ command, args }); return null; } };
    if (name === "@tauri-apps/api/event") return { listen: async (event) => { events.push(event); return () => {}; } };
    throw new Error(`Unexpected dependency: ${name}`);
  },
});
const api = module.exports.api;
await api.inspectTorrent({ torrentPath: "C:/Audit/synthetic.torrent", torrentBase64: "U1lOVEhFVElD" });
await api.refreshTaskUrl("synthetic-task", "https://example.invalid/new");
await api.updateBtTrackers("https://example.invalid/trackers");
await api.probeMedia("https://example.invalid/video", { user_agent:"SyntheticAgent", include_collection:true });
await api.subscribeSettings(() => {});

// 固定命令兼容契约；捕获真实 API 载荷，避免纯源码字符串检查漏掉运行时拼装。
assert.equal(payloads[0].command, "bt_inspect_torrent");
assert.equal(payloads[0].args.torrentPath, "C:/Audit/synthetic.torrent");
assert.equal(payloads[0].args.torrentBase64, "U1lOVEhFVElD");
assert.equal(payloads[1].args.newUrl, "https://example.invalid/new");
assert.equal(payloads[2].args.subscribeUrl, "https://example.invalid/trackers");
assert.equal(payloads[3].args.userAgent, "SyntheticAgent");
assert.equal(payloads[3].args.includeCollection, true);
assert.deepEqual(events, ["settings-changed"]);

const progressSource=await fs.readFile(new URL("src/components/progress/progress-api.ts",root),"utf8");
const progressModule={exports:{}};
vm.runInNewContext(ts.transpileModule(progressSource,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2021}}).outputText,{
  module:progressModule,exports:progressModule.exports,
  require:name=>{
    if(name==="@tauri-apps/api/core")return {invoke:async(command,args)=>{payloads.push({command,args});return null;}};
    if(name==="@tauri-apps/api/event")return {listen:async event=>{events.push(event);return()=>{};}};
    throw new Error(`Unexpected progress dependency: ${name}`);
  },
});
const progress=progressModule.exports.progressApi;
await progress.snapshot();
await progress.pinned(false);
await progress.expanded(true);
await progress.revealed(true);
await progress.action("synthetic-task","pause");
await progress.showMain("synthetic-task");
await progress.dismiss();
await progress.restore();
await progress.subscribe(()=>{});
assert.deepEqual(JSON.parse(JSON.stringify(payloads.slice(4))),[
  {command:"progress_window_snapshot"},
  {command:"progress_window_set_pinned",args:{pinned:false}},
  {command:"progress_window_set_expanded",args:{expanded:true}},
  {command:"progress_window_set_revealed",args:{revealed:true}},
  {command:"progress_window_task_action",args:{taskId:"synthetic-task",action:"pause"}},
  {command:"progress_window_show_main",args:{taskId:"synthetic-task"}},
  {command:"progress_window_dismiss"},
  {command:"progress_window_restore"},
]);
assert.equal(events.at(-1),"download-progress-updated");

const manager = await fs.readFile(new URL("src-tauri/src/manager.rs", root), "utf8");
assert.equal((manager.match(/\.emit\("settings-changed"/g) || []).length, 2,
  "普通保存和备份恢复都必须发送前端监听的设置事件");
assert.ok(manager.includes('.emit("settings-updated"'), "保留旧设置事件兼容入口");
const select = await fs.readFile(new URL("src/components/Select.tsx", root), "utf8");
console.log(JSON.stringify({
  scope: "Actual api.ts executed against isolated invoke/listen substitutes; no native invocation",
  payloads,
  subscribedEvents: events,
  managerSettingsEvent: /\.emit\("(settings-[^"]+)"/.exec(manager)?.[1],
  selectSourceObservations: {
    hasArrowKeyHandling: /ArrowDown|ArrowUp/.test(select),
    hasEscapeHandling: /event\.key === "Escape"/.test(select),
    hasListboxOrOptionRoles: /role=["'](?:listbox|option)["']/.test(select),
    hasExpandedState: /aria-expanded/.test(select),
    hasOptionTabIndex: /tabIndex/.test(select),
  },
}, null, 2));
