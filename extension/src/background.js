import { signedFetch, signedGet, compatFetch, focusDesktop, PROBE_TIMEOUT_MS } from "./protocol.js";
import { interceptBrowserDownload, evaluateDownload, skipUnpairedDownload, notifyThrottled, recoverStuckTakeovers } from "./interceptor.js";
import { bridgeMediaTask } from "./media-selection.js";
import { requestPageWithTrackingFallback } from "./rules.js";
import { buildCookieHeader } from "./auth-download.js";
import { matchMediaDomain } from "./domains.js";
import { extractLinksFromText, isDownloadableMagnet } from "./links.js";
import { attachSniffer, toggleSniffHost } from "./sniffer.js";

const swStartTime = Date.now();
const defaults = {
  intercept: true, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: 0,
  // 接管模式："auto"（默认，短暂浮层倒计时后自动接管）/ "ask"（每次询问，等用户选择）。
  takeoverMode: "auto",
  // auto 模式浮层倒计时（毫秒，0–5000；0 = 立即接管不弹浮层）。
  autoDelayMs: 1500,
  // 字幕语言偏好："all" / "zh" / "none"，作用于页面媒体探测任务的字幕列表。
  subtitlePref: "all",
  // magnet: 链接接管开关（默认开，可关；桌面端设置经 /v1/health 同步生效）。
  interceptMagnet: true,
  // 浮层"记住对此站点的选择"：{ [host]: "take" | "bypass" }。
  siteChoices: {},
  // 流嗅探按站点开关（默认全关，AGENTS.md §5：仅记录用户显式开启站点的媒体直链）。
  snifferHosts: [],
};
const config = async () => ({ ...defaults, ...(await chrome.storage.local.get(Object.keys(defaults))) });

/// 长操作期间保活：MV3 SW 空闲约 30 秒即被回收；周期性自调用重置空闲计时器，
/// 防止 60 秒的 /v1/media/probe 中途死亡导致页面 FAB 停留在"分析中…"。
function withKeepAlive(promise) {
  const timer = setInterval(() => {
    try { void chrome.runtime?.getPlatformInfo?.(); } catch { /* 测试环境无此 API */ }
  }, 20_000);
  return Promise.resolve(promise).finally(() => clearInterval(timer));
}

// 桌面端接管设置缓存（health 的 takeover_enabled / min_file_size_mb / bt_magnet_enabled）。
// SW 生命周期内缓存 30 秒；桌面端离线/请求失败返回 null，此时不阻断——
// 接管尝试会在 sendTask 阶段失败并走离线回退路径。
let desktopGateCache = null;
const DESKTOP_GATE_TTL_MS = 30_000;
async function fetchDesktopGate() {
  const current = Date.now();
  if (desktopGateCache && current - desktopGateCache.at < DESKTOP_GATE_TTL_MS) return desktopGateCache;
  try {
    const response = await compatFetch("/v1/health");
    if (!response.ok) return null;
    const data = await response.json();
    desktopGateCache = {
      at: current,
      enabled: data.takeover_enabled !== false,
      minSizeMb: Number(data.min_file_size_mb || 0),
      btMagnetEnabled: data.bt_magnet_enabled !== false,
    };
    return desktopGateCache;
  } catch {
    return null;
  }
}

/// 测试辅助：清除桌面端设置缓存，避免用例间通过 30 秒 TTL 缓存互相污染。
export function resetDesktopGateCacheForTest() {
  desktopGateCache = null;
}

async function sendTask(url, fileName, extra = {}) {
  const response = await signedFetch("/v1/tasks", {
    url, file_name: fileName || undefined, headers: extra.headers || {}, priority: 0,
    per_task_speed_limit: 0, collision_policy: "rename", source: "browser", media: extra.media,
  });
  if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
  return response.json();
}

// 流嗅探接线（webRequest 不可用的环境返回 null，嗅探功能整体降级关闭）。
const snifferBridge = attachSniffer({ chrome });

// 判断错误是否为桌面端离线/连接失败/无响应（SubTask 13.5 + P0-3）。
// fetch 无法建立 TCP 连接时抛 TypeError，是离线的可靠信号；
// AbortError（P0-3 超时中止）说明桌面端进程僵死，同样走浏览器回退。
// HTTP 4xx/5xx 不算离线，仍按正常错误处理。
export function isDesktopOfflineError(error) {
  const message = String(error?.message || error);
  return error instanceof TypeError
    || /Failed to fetch|NetworkError|fetch failed|ECONNREFUSED|connect ECONN|abort|timed?\s*out/i.test(message);
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: "lumaget-link", title: "使用猫步下载器下载链接", contexts: ["link"] });
    chrome.contextMenus.create({ id: "lumaget-media", title: "使用猫步下载器下载媒体", contexts: ["video", "audio", "image"] });
    chrome.contextMenus.create({ id: "lumaget-page", title: "使用猫步下载器分析当前页面", contexts: ["page"] });
    chrome.contextMenus.create({ id: "lumaget-selection", title: "使用猫步下载器下载选中文字中的链接", contexts: ["selection"] });
  });
});

// ---- 页面媒体发送（右键"分析当前页面"、悬浮按钮 page 模式、快捷键共用） ----
// 附带当前页 Cookie：B 站等平台风控要求 buvid 等 Cookie，裸请求会被 412 拦截。
// 一次性传递给探针，不持久化（凭证库同步由常规浏览流程负责）。
// `cookieStoreId`：无痕窗口的 Cookie 存放在独立 store，必须显式指定才能读到。
async function sendPageMedia(url, title, cookieStoreId) {
  let pageCookie = "";
  try {
    const params = { url };
    if (cookieStoreId) params.storeId = cookieStoreId;
    const cookies = await chrome.cookies.getAll(params);
    pageCookie = buildCookieHeader(cookies || []);
  } catch {}
  const isBili = /bilibili\.com|b23\.tv/i.test(url);
  const response = await withKeepAlive(requestPageWithTrackingFallback(
    (candidate) => signedFetch("/v1/media/probe", {
      url: candidate,
      cookie: pageCookie || undefined,
      referer: isBili ? "https://www.bilibili.com/" : undefined,
    }, { timeoutMs: PROBE_TIMEOUT_MS }),
    url,
  ));
  if (!response.ok) throw new Error(await response.text());
  const { mediaQuality, subtitlePref } = await chrome.storage.local.get(["mediaQuality", "subtitlePref"]);
  const task = bridgeMediaTask(await response.json(), title, mediaQuality || "best", subtitlePref || "all");
  await sendTask(url, task.fileName, { media: task.media });
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  // 选中文字链接：提取磁力与 HTTP(S) 链接（最多 5 条）。
  // 单条直接发送；多条先打开预览页勾选确认，避免一次性批量添加任务。
  if (info.menuItemId === "lumaget-selection") {
    const { magnets, urls } = extractLinksFromText(info.selectionText || "", 5);
    const links = [...magnets, ...urls];
    if (!links.length) {
      notify("未发现可下载链接", "选中文本中没有磁力或 HTTP(S) 链接");
      return;
    }
    if (links.length === 1) {
      try {
        await sendTask(links[0]);
        notify("已发送到猫步下载器", tab?.title || links[0]);
      } catch (error) { notify("发送失败", String(error.message || error)); }
      return;
    }
    let opened = null;
    try {
      opened = await chrome.windows?.create?.({
        url: `src/selection.html#${encodeURIComponent(JSON.stringify(links))}`,
        type: "popup", width: 500, height: 440,
      });
    } catch { /* windows API 不可用时退回逐条发送。 */ }
    if (opened) return;
    let added = 0;
    let failed = 0;
    for (const link of links) {
      try { await sendTask(link); added += 1; } catch { failed += 1; }
    }
    notify(added ? "已发送到猫步下载器" : "发送失败",
      `${added} 个任务已添加${failed ? `，${failed} 个失败` : ""}`);
    return;
  }
  const url = info.linkUrl || info.srcUrl || info.pageUrl;
  if (!url) return;
  try {
    if (info.menuItemId === "lumaget-page") {
      await sendPageMedia(url, tab?.title, tab?.cookieStoreId);
    } else {
      // 右键"下载链接/下载媒体"：src 直链直接交给桌面端下载（对图片、直链
      // 视频/音频最可靠）；MSE 站点走"分析当前页面"或页内悬浮按钮。
      await sendTask(url);
    }
    notify("已发送到猫步下载器", tab?.title || url);
  }
  catch (error) { notify("发送失败", String(error.message || error)); }
});

chrome.downloads.onCreated.addListener(async (item) => {
  const settings = await config();
  // 桌面端接管设置接线：桌面端"设置 → 浏览器"中的开关与最小体积阈值
  // 通过 health 端点同步，扩展侧取两者中更严格的限制。
  const desktopGate = await fetchDesktopGate();
  if (desktopGate) {
    settings.desktopTakeoverEnabled = desktopGate.enabled;
    if (desktopGate.minSizeMb > 0) {
      settings.minSizeMb = Math.max(Number(settings.minSizeMb || 0), desktopGate.minSizeMb);
    }
  }
  const evalResult = evaluateDownload(item, settings, chrome.runtime.id, swStartTime);
  if (!evalResult.eligible) {
    try { await chrome.downloads.resume(item.id); } catch {}
    return;
  }
  // 配对预检：未配对时不进入浮层与接管流程，直接由浏览器下载。
  // 避免未配对状态下每个下载都弹浮层 + 重复"接管失败"通知。
  if (await skipUnpairedDownload(item, notify)) {
    try { await chrome.downloads.resume(item.id); } catch {}
    return;
  }
  const tab = await findSourceTab(item);
  const proceed = await confirmTakeoverWithOverlay(item, settings, { tab });
  if (!proceed) {
    try { await chrome.downloads.resume(item.id); } catch {}
    return;
  }
  const handled = await interceptBrowserDownload(item, {
    downloads: chrome.downloads, settings, runtimeId: chrome.runtime.id, sendTask, notify,
    isDesktopOfflineError, swStartTime,
    // P2-11：接管成功后在源页面显示徽章；页面不可达时退回可点击系统通知。
    // created（含任务 id）用于徽章上的"撤销"按钮（取消桌面端任务）。
    onTakenOver: (decision, created) => { void announceTakeover(tab, decision?.fileName || "", created?.id); },
  });
  if (!handled) {
    try { await chrome.downloads.resume(item.id); } catch {}
  }
});

async function announceTakeover(tab, fileName, taskId) {
  if (tab?.id != null) {
    try {
      await chrome.tabs.sendMessage(tab.id, { type: "show-badge", kind: "added", fileName, taskId });
      return;
    } catch {}
  }
  await notifyThrottled("added", "已添加到猫步下载器", fileName || "点击查看任务", notify, "takeover-added");
}

// SubTask 13.4 + P2-10/P2-13：接管确认浮层。
// 流程：
//   1. 通过 item.referrer / finalUrl 定位源 tab；找不到则直接接管（不阻塞用户）。
//   2. 向 content script 发送 show-overlay 消息（带模式：auto/ask、倒计时毫秒、
//      文件大小与来源主机，供浮层展示更完整的决策信息）。
//      auto：倒计时（默认 1.5 秒，用户可调 0–5000）后自动接管；ask：等待用户
//      选择，20 秒超时放行浏览器。delayMs=0 时不弹浮层直接接管。
//   3. 用户点击"绕过"返回 { bypass: true }；"记住选择"由 content script 的
//      事后轻提示完成（兼容旧浮层的 remember 字段）。
//   4. content script 不可达（如 chrome:// 页面、PDF viewer）时直接接管。
const clampAutoDelayMs = (value) => Math.max(0, Math.min(5000, Math.round(Number(value ?? 1500)) || 0));

export async function confirmTakeoverWithOverlay(item, settings, deps = {}) {
  const sendMessage = deps.sendMessage || ((tabId, msg) => chrome.tabs.sendMessage(tabId, msg));
  const notifyFn = deps.notify || notify;
  if (!settings.intercept) return true;
  if (Date.now() < Number(settings.bypassUntil || 0)) return true;
  const runtimeId = deps.runtimeId || chrome.runtime?.id;
  const swTime = deps.swStartTime || swStartTime;
  const evalResult = evaluateDownload(item, settings, runtimeId, swTime);
  if (!evalResult.eligible) return false;
  const tab = deps.tab || await findSourceTab(item, deps);
  if (!tab) return true;
  const mode = settings.takeoverMode === "ask" ? "ask" : "auto";
  const delayMs = clampAutoDelayMs(settings.autoDelayMs);
  if (mode === "auto" && delayMs <= 0) return true; // 0 秒 = 总是接管，不打扰。
  let downloadHost = "";
  try { downloadHost = new URL(item.finalUrl || item.url || "").hostname; } catch {}
  let response = null;
  const askOverlay = async () => sendMessage(tab.id, {
    type: "show-overlay",
    fileName: item.filename || "",
    mode,
    delayMs,
    sizeBytes: Number(item.totalBytes || 0),
    host: downloadHost,
  });
  try {
    response = await askOverlay();
  } catch {
    try {
      if (chrome.scripting?.executeScript) {
        await chrome.scripting.executeScript({ target: { tabId: tab.id }, files: ["src/content-ui.js", "src/content.js"] });
        response = await askOverlay();
      }
    } catch {}
  }
  if (!response) return true;
  if (response.remember && response.host) {
    await rememberSiteChoice(response.host, response.bypass ? "bypass" : "take");
  }
  if (response.bypass) {
    // 用户能点到绕过按钮说明正看着页面：浮层关闭 + "记住选择"轻提示已是
    // 明确反馈，不再叠加系统通知（减少打扰）。
    if (response.remember && response.host) {
      notifyFn("已记住：此站点由浏览器下载", "可在扩展弹窗中清除站点记忆");
    }
    return false;
  }
  return true;
}

/// 写入站点记忆选择；config() 每次下载都重新读存储，立即生效。
async function rememberSiteChoice(host, value) {
  try {
    const { siteChoices = {} } = await chrome.storage.local.get("siteChoices");
    siteChoices[host] = value;
    await chrome.storage.local.set({ siteChoices });
  } catch {}
}

const originOf = (url) => {
  try { return new URL(url).origin; } catch { return ""; }
};

/// 定位下载的来源标签页。
///
/// 优先级：当前活动标签（若与下载 referrer 同源，或无 referrer）→
/// 与 referrer 同源的任意标签（后台标签/其它窗口发起的下载也能正确投放
/// 浮层与成功徽章）→ 当前活动标签 → 任意活动标签。
export async function findSourceTab(item, deps = {}) {
  const queryTabs = deps.queryTabs || ((q) => chrome.tabs.query(q));
  const referrerOrigin = originOf(item?.referrer || "");
  let activeCurrent = null;
  try {
    const tabs = await queryTabs({ active: true, currentWindow: true });
    if (tabs && tabs[0] && /^https?:/i.test(tabs[0].url || "")) activeCurrent = tabs[0];
  } catch {}
  if (activeCurrent && (!referrerOrigin || originOf(activeCurrent.url) === referrerOrigin)) {
    return activeCurrent;
  }
  if (referrerOrigin) {
    let all = [];
    try { all = (await queryTabs({})) || []; } catch {}
    const matched = all.filter((tab) => /^https?:/i.test(tab.url || "") && originOf(tab.url) === referrerOrigin);
    const activeMatched = matched.find((tab) => tab.active) || matched[0];
    if (activeMatched) return activeMatched;
  }
  if (activeCurrent) return activeCurrent;
  try {
    const tabs = await queryTabs({ active: true });
    if (tabs && tabs[0] && /^https?:/i.test(tabs[0].url || "")) return tabs[0];
  } catch {}
  return null;
}

// 桥接错误转用户可读文案：剥离桌面端内部前缀，补上可操作指引。
function friendlyBridgeError(error) {
  const message = String(error?.message || error);
  if (message.startsWith("MEDIA_YT_DLP_MISSING:")) {
    return `${message.slice("MEDIA_YT_DLP_MISSING:".length).trim()}。请打开猫步下载器 → 设置 → 媒体工具，安装基础组件后重试。`;
  }
  return message;
}

function notify(title, message, notificationId) {
  // 图标路径相对扩展根目录解析（构建产物中位于 src/ 下）。
  const options = { type: "basic", iconUrl: "src/icon128.png", title, message };
  if (notificationId) {
    chrome.notifications.create(notificationId, options);
  } else {
    chrome.notifications.create(options);
  }
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
      // 透传桌面端版本号：popup 用于比较扩展版本并提示更新/重载。
      const desktopVersion = await response.json().then((data) => String(data?.version || "")).catch(() => "");
      const stored = await chrome.storage.local.get("bridgeToken");
      if (!stored.bridgeToken) return { ok: true, paired: false, version: desktopVersion };
      try {
        const checkRes = await signedGet("/v1/tasks/recent");
        if (!checkRes.ok) {
          if (checkRes.status === 401) {
            await chrome.storage.local.remove("bridgeToken").catch(() => {});
            return { ok: true, paired: false, version: desktopVersion };
          }
        }
      } catch {
        const storedAfter = await chrome.storage.local.get("bridgeToken");
        if (!storedAfter.bridgeToken) {
          return { ok: true, paired: false, version: desktopVersion };
        }
      }
      const hasToken = Boolean((await chrome.storage.local.get("bridgeToken")).bridgeToken);
      return { ok: true, paired: hasToken, version: desktopVersion };
    }
    if (message.type === "send") return { ok: true, item: await sendTask(message.url, message.fileName, message.extra) };
    // 页内悬浮按钮（page 模式）：MSE 站点（B 站/YouTube 等）拿不到 http(s) 直链，
    // 发送页面 URL，与右键菜单"使用猫步下载器下载媒体"走相同的探测与任务构造流程。
    if (message.type === "download-page-media") {
      try {
        await sendPageMedia(message.url, message.title || sender.tab?.title, sender.tab?.cookieStoreId);
        return { ok: true };
      } catch (error) {
        // 完整错误用系统通知展示（悬浮按钮空间有限只显示截断文案），
        // 内部前缀（如 MEDIA_YT_DLP_MISSING）翻译为带安装指引的中文。
        const friendly = friendlyBridgeError(error);
        notify("猫步下载器发送失败", friendly);
        return { ok: false, error: friendly };
      }
    }
    if (message.type === "probe") { const response = await signedFetch("/v1/media/probe", { url: message.url }, { timeoutMs: PROBE_TIMEOUT_MS }); if (!response.ok) throw new Error(await response.text()); return { ok: true, result: await response.json() }; }
    // magnet: 链接接管（BT-08）：content script 拦截页面磁力点击后发来。
    // 任何前置条件不满足（扩展开关/桌面端设置/配对/离线）都返回 fallback: true，
    // 由 content script 把链接交还浏览器原生处理（§5 离线回退，不丢失用户下载）。
    if (message.type === "send-magnet") {
      const url = String(message.url || "");
      if (!isDownloadableMagnet(url)) return { ok: false, fallback: true, error: "不是有效的磁力链接" };
      const settings = await config();
      if (!settings.interceptMagnet) return { ok: false, fallback: true };
      const gate = await fetchDesktopGate();
      if (gate && gate.btMagnetEnabled === false) return { ok: false, fallback: true };
      const { bridgeToken } = await chrome.storage.local.get("bridgeToken").catch(() => ({}));
      if (!bridgeToken) {
        await notifyThrottled(
          "unpaired",
          "尚未与桌面端配对，磁力已交还浏览器处理",
          "打开猫步下载器 → 设置 → 浏览器，输入配对码完成配对",
          notify,
          "unpaired-warning"
        );
        return { ok: false, fallback: true };
      }
      try {
        await sendTask(url);
        return { ok: true };
      } catch (error) {
        if (isDesktopOfflineError(error)) {
          await notifyThrottled("offline", "桌面端离线，磁力已交还浏览器处理", "点击此处打开猫步下载器", notify, "offline-warning");
          return { ok: false, fallback: true, offline: true };
        }
        const friendly = friendlyBridgeError(error);
        return { ok: false, fallback: false, error: friendly };
      }
    }
    if (message.type === "bypass") {
      if (message.cancel) {
        await chrome.storage.local.set({ bypassUntil: 0 });
        return { ok: true, active: false };
      }
      const until = Date.now() + Number(message.minutes || 10) * 60_000;
      await chrome.storage.local.set({ bypassUntil: until });
      return { ok: true, active: true, until };
    }
    // 流嗅探：popup 查询当前标签的嗅探记录与站点开关状态。
    if (message.type === "sniffed-media") {
      const items = snifferBridge ? snifferBridge.getItems(Number(message.tabId)) : [];
      return { ok: true, items, enabled: Boolean(snifferBridge?.isHostEnabled(message.host)) };
    }
    // 流嗅探站点开关：popup 为当前站点一键开启/关闭。
    if (message.type === "sniff-toggle") {
      const host = String(message.host || "").toLowerCase();
      if (!host) return { ok: false, error: "无效域名" };
      const { snifferHosts = [] } = await chrome.storage.local.get("snifferHosts");
      const hosts = toggleSniffHost(snifferHosts, host, Boolean(message.enabled));
      await chrome.storage.local.set({ snifferHosts: hosts });
      return { ok: true, enabled: Boolean(message.enabled), hosts };
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

// ---- 通知点击闭环（P0-1）----
// 所有带固定 ID 的通知被点击时唤起桌面端主窗口（POST /v1/focus）；
// 桌面端离线唤起失败时尝试打开扩展弹窗，给出配对/连接引导。
// 此前离线通知文案写着"点击此处打开猫步下载器"但没有任何点击监听，
// 点击毫无反应——本监听补上这一闭环。
const CLICKABLE_NOTIFICATIONS = new Set([
  "offline-warning",
  "unpaired-warning",
  "takeover-error",
  "takeover-cancel-error",
  "takeover-added",
]);
chrome.notifications.onClicked?.addListener?.((notificationId) => {
  if (!CLICKABLE_NOTIFICATIONS.has(notificationId)) return;
  void (async () => {
    // 未配对时 focusDesktop（需签名）必然失败，直接打开扩展弹窗给配对引导。
    let bridgeToken = null;
    try { ({ bridgeToken } = await chrome.storage.local.get("bridgeToken")); } catch {}
    const focused = bridgeToken ? await focusDesktop() : false;
    if (!focused) {
      try { await chrome.action.openPopup(); } catch { /* Chrome <127 不支持编程打开弹窗 */ }
    }
  })();
});

// ---- 接管看门狗（P0-4）----
// 每分钟检查 storage.session 中超龄的待接管标记：SW 崩溃/被回收导致下载
// 永久卡在暂停态时，由这里恢复放行。SW 启动时也立即检查一次。
const WATCHDOG_ALARM = "takeover-watchdog";
try {
  chrome.alarms?.create?.(WATCHDOG_ALARM, { periodInMinutes: 1 });
  chrome.alarms?.onAlarm?.addListener?.((alarm) => {
    if (alarm?.name === WATCHDOG_ALARM) void recoverStuckTakeovers();
  });
} catch { /* alarms 不可用（旧浏览器/测试环境）时退化为 SW 启动时检查。 */ }
void recoverStuckTakeovers();

// ---- 快捷键（P3-19）----
// toggle-takeover：临时暂停/恢复接管 10 分钟（与弹窗按钮同一存储位）。
// send-page-media：把当前页媒体发送到桌面端（与悬浮按钮 page 模式一致）。
chrome.commands?.onCommand?.addListener?.((command) => {
  void (async () => {
    if (command === "toggle-takeover") {
      const { bypassUntil } = await chrome.storage.local.get("bypassUntil").catch(() => ({}));
      if (Number(bypassUntil || 0) > Date.now()) {
        await chrome.storage.local.set({ bypassUntil: 0 });
        notify("已恢复接管", "浏览器下载将继续交给猫步下载器");
      } else {
        await chrome.storage.local.set({ bypassUntil: Date.now() + 10 * 60_000 });
        notify("接管已暂停 10 分钟", "期间下载由浏览器处理");
      }
      return;
    }
    if (command === "send-page-media") {
      try {
        const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tab?.url || !/^https?:/i.test(tab.url)) {
          notify("无法发送页面媒体", "当前标签页不是 HTTP/HTTPS 页面");
          return;
        }
        await sendPageMedia(tab.url, tab.title, tab.cookieStoreId);
        notify("已发送到猫步下载器", tab.title || tab.url);
      } catch (error) {
        notify("猫步下载器发送失败", friendlyBridgeError(error));
      }
    }
  })();
});

// ---- 媒体平台 Cookie 同步（P1-7）----
// 节流时间戳持久化到 storage.session：SW 被回收后仍生效，
// 避免每次 SW 重启都对所有打开标签页重复同步。
// 域名表统一来自 domains.js（此前 background 与 popup 各复制一份）。
const SYNC_INTERVAL_MS = 5 * 60_000;

async function shouldSyncNow(baseDomain) {
  try {
    const session = chrome.storage?.session;
    if (!session?.get || !session?.set) return true;
    const { mediaSyncLast = {} } = await session.get("mediaSyncLast");
    const now = Date.now();
    if (now - Number(mediaSyncLast[baseDomain] || 0) <= SYNC_INTERVAL_MS) return false;
    mediaSyncLast[baseDomain] = now;
    await session.set({ mediaSyncLast });
    return true;
  } catch {
    return true; // storage 暂不可用时宁可多同步一次，不丢凭证。
  }
}

async function syncCookiesForDomain(domain, url, cookieStoreId) {
  try {
    const stored = await chrome.storage.local.get("bridgeToken");
    if (!stored.bridgeToken) return;
    // 无痕窗口的 Cookie 在独立 store 中，必须显式指定 storeId 才能读到。
    const params = { url };
    if (cookieStoreId) params.storeId = cookieStoreId;
    const cookies = await chrome.cookies.getAll(params);
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

async function maybeSyncTabCookies(tab) {
  try {
    const hostname = new URL(tab.url).hostname.toLowerCase();
    const { customMediaDomains = [] } = await chrome.storage.local.get("customMediaDomains");
    const baseDomain = matchMediaDomain(hostname, customMediaDomains);
    if (baseDomain && await shouldSyncNow(baseDomain)) {
      await syncCookiesForDomain(baseDomain, tab.url, tab.cookieStoreId);
    }
  } catch {}
}

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.status === "complete" && tab.url) {
    void maybeSyncTabCookies(tab);
  }
});

async function syncAllOpenTabs() {
  try {
    const tabs = await chrome.tabs.query({});
    for (const tab of tabs) {
      if (tab.url) await maybeSyncTabCookies(tab);
    }
  } catch (err) {
    console.error("Failed to query open tabs on startup:", err);
  }
}

// Call on startup
void syncAllOpenTabs();
