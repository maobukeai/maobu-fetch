// 接线一致性回归测试（纯静态，无 DOM/Chrome API 依赖）。
//
// popup.js / options.js / selection.js 没有 DOM 单测，元素 id 拼错或消息类型
// 改名不同步时功能会静默失效。本文件把三类交叉契约固化为断言：
//   1. JS 引用的元素 id 必须存在于对应 HTML；
//   2. 扩展各方向发送的消息类型必须有接收方处理；
//   3. manifest 的 content_scripts 与 background 兜底注入文件列表保持一致。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const read = (name) => readFileSync(join(__dirname, name), "utf8");

// ---- 1. 元素 id 接线 ----

function htmlIds(html) {
  const ids = new Set();
  for (const match of html.matchAll(/\sid="([^"]+)"/g)) ids.add(match[1]);
  return ids;
}

/// 提取 `$("id")` 与 `getElementById("id")` 的字面量引用（变量形式的引用
/// 由下方显式清单覆盖）。
function literalElementIds(js) {
  const ids = new Set();
  for (const match of js.matchAll(/\$\("([^"]+)"\)/g)) ids.add(match[1]);
  for (const match of js.matchAll(/getElementById\("([^"]+)"\)/g)) ids.add(match[1]);
  return ids;
}

test("wiring: popup.js 引用的元素 id 都存在于 popup.html", () => {
  const ids = htmlIds(read("popup.html"));
  const referenced = literalElementIds(read("popup.js"));
  assert.ok(referenced.size > 20, "应提取到足量引用，正则失效时兜底失败");
  for (const id of referenced) {
    assert.ok(ids.has(id), `popup.html 缺少 #${id}（popup.js 引用但页面不存在）`);
  }
});

test("wiring: options.js 的字段/hint id 都存在于 options.html", () => {
  const ids = htmlIds(read("options.html"));
  for (const id of literalElementIds(read("options.js"))) {
    assert.ok(ids.has(id), `options.html 缺少 #${id}`);
  }
  // options.js 通过变量引用的字段与 hint（fields 数组 + validateLive 检查表）。
  for (const id of [
    "allowHosts", "blockHosts", "extensions", "snifferHosts", "customMediaDomains",
    "allowHostsHint", "blockHostsHint", "extensionsHint", "snifferHostsHint", "customMediaDomainsHint",
    "testUrl", "testFile", "testSize", "testResult", "runTest", "interceptMagnet",
    "importRules", "exportRules", "importFile", "saveRules", "message",
  ]) {
    assert.ok(ids.has(id), `options.html 缺少 #${id}`);
  }
});

test("wiring: selection.js 引用的元素 id 都存在于 selection.html", () => {
  const ids = htmlIds(read("selection.html"));
  for (const id of ["selectionList", "status", "send", "selectAll", "linkCount", "cancel", "searchFilter", "categoryChips", "invertSelection", "filteredHint"]) {
    assert.ok(ids.has(id), `selection.html 缺少 #${id}`);
  }
});

// ---- 2. 消息契约 ----

// background.onMessage 必须处理的消息类型（popup 的 call / content 的
// sendMessageSafe 全部发往 background）。
const BACKGROUND_HANDLED = [
  "media", "pair", "health", "send", "download-page-media", "probe", "send-magnet",
  "bypass", "sniffed-media", "sniff-toggle", "recent-tasks", "task-action", "sync-cookies",
  "grab-page-resources",
];
// content script 必须处理的消息类型（background 经 tabs.sendMessage 推送）。
const CONTENT_HANDLED = ["show-overlay", "show-badge", "sniffed-media", "grab-page-resources"];

test("wiring: background 处理全部已知扩展消息类型", () => {
  const bg = read("background.js");
  for (const type of BACKGROUND_HANDLED) {
    assert.ok(bg.includes(`message.type === "${type}"`), `background.js 缺少 "${type}" 处理分支`);
  }
});

test("wiring: content script 处理 background 推送的消息类型", () => {
  const content = read("content.js");
  for (const type of CONTENT_HANDLED) {
    assert.ok(content.includes(`message?.type === "${type}"`), `content.js 缺少 "${type}" 处理分支`);
  }
});

test("wiring: popup/content 内联发送的消息类型都有 background 接收方", () => {
  const known = new Set(BACKGROUND_HANDLED);
  const sentTypes = (js, senderPattern) => {
    const types = new Set();
    const pattern = new RegExp(`${senderPattern}\\(\\s*\\{\\s*type:\\s*"([^"]+)"`, "g");
    for (const match of js.matchAll(pattern)) types.add(match[1]);
    return types;
  };
  const fromPopup = sentTypes(read("popup.js"), "call");
  const fromContent = sentTypes(read("content.js"), "sendMessageSafe");
  for (const [source, types] of [["popup.js", fromPopup], ["content.js", fromContent]]) {
    for (const type of types) {
      assert.ok(known.has(type), `${source} 发送的 "${type}" 在 background.js 没有处理分支`);
    }
  }
});

// ---- 3. 注入文件清单一致性 ----

test("wiring: manifest content_scripts 与 background 兜底注入保持一致", () => {
  const manifest = JSON.parse(read("../manifest.json"));
  const manifestFiles = manifest.content_scripts[0].js;
  const bg = read("background.js");
  const injectMatch = bg.match(/files:\s*\[([^\]]+)\]/);
  assert.ok(injectMatch, "background.js 应包含 executeScript files 列表");
  const bgFiles = [...injectMatch[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(bgFiles, manifestFiles, "兜底注入与 manifest 的注入顺序/文件必须一致");
  for (const file of manifestFiles) {
    assert.ok(existsSync(join(__dirname, file.replace(/^src\//, ""))), `注入文件不存在：${file}`);
  }
});
