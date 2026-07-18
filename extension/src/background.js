const API = "http://127.0.0.1:17433";

async function settings() {
  return { intercept: true, minSizeMb: 1, ...(await chrome.storage.local.get(["intercept", "minSizeMb"])) };
}

async function sendToApp(url, fileName) {
  const response = await fetch(`${API}/downloads`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ url, fileName }) });
  if (!response.ok) throw new Error(await response.text());
  return response.json();
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "lumaget-link", title: "使用 LumaGet 下载", contexts: ["link"] });
  chrome.contextMenus.create({ id: "lumaget-media", title: "使用 LumaGet 下载媒体", contexts: ["video", "audio", "image"] });
  chrome.contextMenus.create({ id: "lumaget-page", title: "发送当前页面到 LumaGet", contexts: ["page"] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl || info.srcUrl || info.pageUrl;
  if (!url) return;
  try { await sendToApp(url); notify("已发送到 LumaGet", tab?.title || url); }
  catch { notify("LumaGet 未连接", "请先打开桌面应用，然后重试。"); }
});

chrome.downloads.onCreated.addListener(async item => {
  const config = await settings();
  if (!config.intercept || !/^https?:/.test(item.url) || item.byExtensionId === chrome.runtime.id) return;
  const minBytes = Number(config.minSizeMb || 1) * 1024 * 1024;
  if (item.totalBytes > 0 && item.totalBytes < minBytes) return;
  try {
    await sendToApp(item.finalUrl || item.url, item.filename?.split(/[\\/]/).pop());
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
  } catch { /* Keep the browser download when the desktop bridge is unavailable. */ }
});

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (message.type === "media") {
    const key = `media:${sender.tab?.id}`;
    chrome.storage.session.set({ [key]: message.items });
  }
  if (message.type === "send") sendToApp(message.url, message.fileName).then(x => respond({ ok: true, item: x })).catch(error => respond({ ok: false, error: error.message }));
  if (message.type === "health") fetch(`${API}/health`).then(r => respond({ ok: r.ok })).catch(() => respond({ ok: false }));
  return true;
});

function notify(title, message) {
  chrome.notifications.create({ type: "basic", iconUrl: "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='128' height='128'><rect rx='32' width='128' height='128' fill='%237866f6'/></svg>", title, message });
}

