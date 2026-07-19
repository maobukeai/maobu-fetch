import { API, signedFetch, compatFetch } from "./protocol.js";
import { interceptBrowserDownload } from "./interceptor.js";
import { bridgeMediaTask } from "./media-selection.js";
import { requestPageWithTrackingFallback } from "./rules.js";

const defaults = { intercept: true, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: 0 };
const config = async () => ({ ...defaults, ...(await chrome.storage.local.get(Object.keys(defaults))) });

async function sendTask(url, fileName, extra = {}) {
  const response = await signedFetch("/v1/tasks", {
    url, file_name: fileName || undefined, headers: extra.headers || {}, priority: 0,
    per_task_speed_limit: 0, collision_policy: "rename", source: "browser", media: extra.media,
  });
  if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
  return response.json();
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: "lumaget-link", title: "使用猫步下载器下载链接", contexts: ["link"] });
    chrome.contextMenus.create({ id: "lumaget-media", title: "使用猫步下载器下载媒体", contexts: ["video", "audio", "image"] });
    chrome.contextMenus.create({ id: "lumaget-page", title: "使用猫步下载器分析当前页面", contexts: ["page"] });
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl || info.srcUrl || info.pageUrl;
  if (!url) return;
  try {
    if (info.menuItemId === "lumaget-page") {
      const response = await requestPageWithTrackingFallback(
        (candidate) => signedFetch("/v1/media/probe", { url: candidate }),
        url,
      );
      if (!response.ok) throw new Error(await response.text());
      const task = bridgeMediaTask(await response.json(), tab?.title);
      await sendTask(url, task.fileName, { media: task.media });
    } else {
      await sendTask(url);
    }
    notify("已发送到猫步下载器", tab?.title || url);
  }
  catch (error) { notify("发送失败", String(error.message || error)); }
});

chrome.downloads.onCreated.addListener(async (item) => {
  const settings = await config();
  await interceptBrowserDownload(item, {
    downloads: chrome.downloads, settings, runtimeId: chrome.runtime.id, sendTask, notify,
  });
});

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  (async () => {
    if (message.type === "media") { await chrome.storage.session.set({ [`media:${sender.tab?.id}`]: message.items }); return { ok: true }; }
    if (message.type === "pair") {
      const response = await compatFetch("/v1/pair", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ code: message.code, extension_id: chrome.runtime.id }) });
      if (!response.ok) throw new Error(await response.text()); const result = await response.json();
      await chrome.storage.local.set({ bridgeToken: result.token }); return { ok: true };
    }
    if (message.type === "health") { const response = await compatFetch("/v1/health"); const stored = await chrome.storage.local.get("bridgeToken"); return { ok: response.ok, paired: Boolean(stored.bridgeToken) }; }
    if (message.type === "send") return { ok: true, item: await sendTask(message.url, message.fileName, message.extra) };
    if (message.type === "probe") { const response = await signedFetch("/v1/media/probe", { url: message.url }); if (!response.ok) throw new Error(await response.text()); return { ok: true, result: await response.json() }; }
    if (message.type === "bypass") { await chrome.storage.local.set({ bypassUntil: Date.now() + Number(message.minutes || 10) * 60_000 }); return { ok: true }; }
    return { ok: false, error: "未知请求" };
  })().then(respond).catch((error) => respond({ ok: false, error: error.message || String(error) }));
  return true;
});

function notify(title, message) {
  chrome.notifications.create({ type: "basic", iconUrl: "icon128.png", title, message });
}
