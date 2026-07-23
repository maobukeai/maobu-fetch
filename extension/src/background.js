import { API, signedFetch, signedGet, compatFetch } from "./protocol.js";
import { interceptBrowserDownload } from "./interceptor.js";
import { bridgeMediaTask } from "./media-selection.js";
import { requestPageWithTrackingFallback } from "./rules.js";
import { buildCookieHeader } from "./auth-download.js";

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

// 判断错误是否为桌面端离线/连接失败（SubTask 13.5）。
// fetch 在无法建立 TCP 连接时会抛出 TypeError，是离线回退的可靠信号；
// HTTP 4xx/5xx 不算离线，仍按正常错误处理。
export function isDesktopOfflineError(error) {
  const message = String(error?.message || error);
  return error instanceof TypeError
    || /Failed to fetch|NetworkError|fetch failed|ECONNREFUSED|connect ECONN/i.test(message);
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
  const proceed = await confirmTakeoverWithOverlay(item, settings);
  if (!proceed) {
    try { await chrome.downloads.resume(item.id); } catch {}
    return;
  }
  const handled = await interceptBrowserDownload(item, {
    downloads: chrome.downloads, settings, runtimeId: chrome.runtime.id, sendTask, notify,
    isDesktopOfflineError,
  });
  if (!handled) {
    try { await chrome.downloads.resume(item.id); } catch {}
  }
});

// SubTask 13.4：通过 content script 在源 tab 显示 1.5 秒浮层。
// 流程：
//   1. 通过 item.referrer / finalUrl 定位源 tab；找不到则直接接管（不阻塞用户）。
//   2. 向 content script 发送 show-overlay 消息，content script 显示浮层并等待 1.5 秒。
//   3. 用户点击"本次绕过"返回 { bypass: true }；超时返回 { bypass: false }。
//   4. content script 不可达（如 chrome:// 页面、PDF viewer）时直接接管。
export async function confirmTakeoverWithOverlay(item, settings, deps = {}) {
  const sendMessage = deps.sendMessage || ((tabId, msg) => chrome.tabs.sendMessage(tabId, msg));
  const notifyFn = deps.notify || notify;
  if (!settings.intercept) return true;
  if (Date.now() < Number(settings.bypassUntil || 0)) return true;
  const tab = await findSourceTab(item, deps);
  if (!tab) return true;
  try {
    const response = await sendMessage(tab.id, { type: "show-overlay", fileName: item.filename || "" });
    if (response && response.bypass) {
      notifyFn("已临时绕过接管", "本次下载将由浏览器处理");
      return false;
    }
  } catch {
    try {
      if (chrome.scripting?.executeScript) {
        await chrome.scripting.executeScript({ target: { tabId: tab.id }, files: ["src/content.js"] });
        const response = await sendMessage(tab.id, { type: "show-overlay", fileName: item.filename || "" });
        if (response && response.bypass) {
          notifyFn("已临时绕过接管", "本次下载将由浏览器处理");
          return false;
        }
      }
    } catch {}
  }
  return true;
}

export async function findSourceTab(item, deps = {}) {
  const queryTabs = deps.queryTabs || ((q) => chrome.tabs.query(q));
  try {
    const tabs = await queryTabs({ active: true, currentWindow: true });
    if (tabs && tabs[0] && /^https?:/i.test(tabs[0].url || "")) return tabs[0];
  } catch {}
  try {
    const tabs = await queryTabs({ active: true });
    if (tabs && tabs[0] && /^https?:/i.test(tabs[0].url || "")) return tabs[0];
  } catch {}
  return null;
}

function sameOrigin(a, b) {
  try { return new URL(a).origin === new URL(b).origin; } catch { return false; }
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  (async () => {
    if (message.type === "media") { await chrome.storage.session.set({ [`media:${sender.tab?.id}`]: message.items }); return { ok: true }; }
    if (message.type === "pair") {
      const response = await compatFetch("/v1/pair", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ code: message.code, extension_id: chrome.runtime.id }) });
      if (!response.ok) throw new Error(await response.text()); const result = await response.json();
      await chrome.storage.local.set({ bridgeToken: result.token }); return { ok: true };
    }
    if (message.type === "health") {
      const response = await compatFetch("/v1/health");
      if (!response.ok) return { ok: false, paired: false };
      const stored = await chrome.storage.local.get("bridgeToken");
      if (!stored.bridgeToken) return { ok: true, paired: false };
      try {
        const checkRes = await signedGet("/v1/tasks/recent");
        if (!checkRes.ok) {
          if (checkRes.status === 401) {
            await chrome.storage.local.remove("bridgeToken").catch(() => {});
            return { ok: true, paired: false };
          }
        }
      } catch {
        const storedAfter = await chrome.storage.local.get("bridgeToken");
        if (!storedAfter.bridgeToken) {
          return { ok: true, paired: false };
        }
      }
      const hasToken = Boolean((await chrome.storage.local.get("bridgeToken")).bridgeToken);
      return { ok: true, paired: hasToken };
    }
    if (message.type === "send") return { ok: true, item: await sendTask(message.url, message.fileName, message.extra) };
    if (message.type === "probe") { const response = await signedFetch("/v1/media/probe", { url: message.url }); if (!response.ok) throw new Error(await response.text()); return { ok: true, result: await response.json() }; }
    if (message.type === "bypass") {
      if (message.cancel) {
        await chrome.storage.local.set({ bypassUntil: 0 });
        return { ok: true, active: false };
      }
      const until = Date.now() + Number(message.minutes || 10) * 60_000;
      await chrome.storage.local.set({ bypassUntil: until });
      return { ok: true, active: true, until };
    }
    // SubTask 13.1/13.2：弹窗查询最近任务、触发任务操作。
    if (message.type === "recent-tasks") {
      const response = await signedGet("/v1/tasks/recent");
      if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
      return { ok: true, result: await response.json() };
    }
    if (message.type === "task-action") {
      const response = await signedFetch(`/v1/tasks/${encodeURIComponent(message.id)}/action`, { action: message.action });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
      return { ok: true, result: await response.json() };
    }
    if (message.type === "sync-cookies") {
      const response = await signedFetch("/v1/media/credentials/sync", {
        domain: message.domain,
        cookie: message.cookie
      });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
      return { ok: true };
    }
    return { ok: false, error: "未知请求" };
  })().then(respond).catch((error) => respond({ ok: false, error: error.message || String(error) }));
  return true;
});

const domainsToSync = ["douyin.com", "tiktok.com", "bilibili.com", "weibo.com", "weibo.cn", "youtube.com", "twitter.com", "x.com"];
const lastSyncTimes = {};

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.status === "complete" && tab.url) {
    try {
      const url = new URL(tab.url);
      const hostname = url.hostname.toLowerCase();
      const matchedDomain = domainsToSync.find(d => hostname === d || hostname.endsWith("." + d));
      if (matchedDomain) {
        let baseDomain = matchedDomain;
        if (baseDomain === "weibo.cn") baseDomain = "weibo.com";
        if (baseDomain === "x.com") baseDomain = "twitter.com";

        const now = Date.now();
        const lastSync = lastSyncTimes[baseDomain] || 0;
        if (now - lastSync > 5 * 60 * 1000) {
          lastSyncTimes[baseDomain] = now;
          syncCookiesForDomain(baseDomain, tab.url);
        }
      }
    } catch (e) {
      console.error("Tab update error:", e);
    }
  }
});

async function syncCookiesForDomain(domain, url) {
  try {
    const stored = await chrome.storage.local.get("bridgeToken");
    if (!stored.bridgeToken) return;
    const cookies = await chrome.cookies.getAll({ url });
    if (!cookies || cookies.length === 0) return;
    const cookieHeader = buildCookieHeader(cookies);
    if (!cookieHeader) return;

    await signedFetch("/v1/media/credentials/sync", {
      domain,
      cookie: cookieHeader
    });
  } catch (err) {
    console.error(`Failed to sync cookies for ${domain}:`, err);
  }
}

function notify(title, message) {
  chrome.notifications.create({ type: "basic", iconUrl: "icon128.png", title, message });
}

async function syncAllOpenTabs() {
  try {
    const tabs = await chrome.tabs.query({});
    for (const tab of tabs) {
      if (!tab.url) continue;
      try {
        const url = new URL(tab.url);
        const hostname = url.hostname.toLowerCase();
        const matchedDomain = domainsToSync.find(d => hostname === d || hostname.endsWith("." + d));
        if (matchedDomain) {
          let baseDomain = matchedDomain;
          if (baseDomain === "weibo.cn") baseDomain = "weibo.com";
          if (baseDomain === "x.com") baseDomain = "twitter.com";
          await syncCookiesForDomain(baseDomain, tab.url);
        }
      } catch {}
    }
  } catch (err) {
    console.error("Failed to query open tabs on startup:", err);
  }
}

// Call on startup
syncAllOpenTabs().catch(() => {});
