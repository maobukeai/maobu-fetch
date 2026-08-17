import { IGNORED_HISTORY_MAX, pushIgnoredEntry } from "./ignored-history.js";

const HTTP_URL = /^https?:/i;

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const host = (url) => {
  try { return new URL(url).hostname.toLowerCase(); } catch { return ""; }
};
const matchesHost = (hostname, rules) => rules.some((rule) => hostname === rule || hostname.endsWith(`.${rule}`));
const basename = (value = "") => value.split(/[\\/]/).pop() || "";
const extensionFrom = (item, urls) => {
  const candidates = [basename(item.filename), ...urls.map((url) => {
    try { return basename(new URL(url).pathname); } catch { return ""; }
  })];
  for (const candidate of candidates) {
    const match = candidate.match(/\.([a-z0-9]{1,12})$/i);
    if (match) return match[1].toLowerCase();
  }
  return "";
};

/// 解析站点记忆选择（浮层"记住对此站点的选择"）。
///
/// `settings.siteChoices` 形如 `{ "example.com": "take" | "bypass" }`，
/// 匹配规则与主机规则一致（精确或子域）。返回命中的值或 null。
/// "bypass" 放行；"take" 强制接管（跳过 allowHosts/后缀/大小检查，
/// 但不越过 blockHosts、桌面端开关与配对状态等安全限制）。
const resolveSiteChoice = (hosts, choices) => {
  for (const hostname of hosts) {
    for (const [choiceHost, value] of Object.entries(choices || {})) {
      if ((value === "take" || value === "bypass") && matchesHost(hostname, [choiceHost])) {
        return value;
      }
    }
  }
  return null;
};

export function evaluateDownload(item, settings, runtimeId, swStartTime = 0, options = {}) {
  if (!settings.intercept) return { eligible: false, reason: "disabled" };
  // 桌面端"允许浏览器扩展接管下载"关闭时不接管（扩展与桌面端设置取交集；
  // 字段缺失表示桌面端离线或为旧版本，此时不阻断，交由发送失败回退路径兜底）。
  if (settings.desktopTakeoverEnabled === false) return { eligible: false, reason: "desktop-disabled" };
  if (Date.now() < Number(settings.bypassUntil || 0)) return { eligible: false, reason: "bypass" };
  if (item.byExtensionId === runtimeId) return { eligible: false, reason: "self" };
  // 其它扩展（如其它下载管理器）发起的下载不接管，避免双重管理与互相取消。
  if (item.byExtensionId && item.byExtensionId !== runtimeId) {
    return { eligible: false, reason: "other-extension" };
  }

  // 校验浏览器重启/会话恢复载入的历史 DownloadItem：
  // 1. 若 item.startTime 早于扩展 Service Worker 启动时间（容许 2 秒误差距），属于历史任务。
  if (swStartTime && item.startTime) {
    const itemStartTime = new Date(item.startTime).getTime();
    if (!isNaN(itemStartTime) && itemStartTime < swStartTime - 2000) {
      return { eligible: false, reason: "restored-history" };
    }
  }

  // 2. 若 item 带有已有下载进度、被暂停、支持恢复或非 in_progress 状态，属于历史恢复任务，不予以拦截。
  //    重评估（reevaluation，扩展自己 pause() 之后对最新快照的二次评估）必须跳过这组
  //    检查：此时 paused/canResume/bytesReceived 是拦截流程自身造成的，并非历史任务信号；
  //    若不跳过，二次评估必然误判 restored-history，导致接管完全失效。
  if (!options.reevaluation
    && (item.bytesReceived > 0 || item.paused || item.canResume || (item.state && item.state !== "in_progress"))) {
    return { eligible: false, reason: "restored-history" };
  }

  const urls = [...new Set([item.finalUrl, item.url].filter((url) => HTTP_URL.test(url || "")))];
  if (!urls.length) return { eligible: false, reason: "scheme" };
  const hosts = urls.map(host).filter(Boolean);
  if (hosts.some((hostname) => matchesHost(hostname, settings.blockHosts || []))) {
    return { eligible: false, reason: "blocked-host" };
  }
  const siteChoice = resolveSiteChoice(hosts, settings.siteChoices);
  if (siteChoice === "bypass") return { eligible: false, reason: "site-bypass" };
  const forcedTake = siteChoice === "take";

  if (!forcedTake && (settings.allowHosts || []).length && !hosts.some((hostname) => matchesHost(hostname, settings.allowHosts))) {
    return { eligible: false, reason: "not-allowed-host" };
  }
  const extension = extensionFrom(item, urls);
  if (!forcedTake && (settings.extensions || []).length && !settings.extensions.includes(extension)) {
    return { eligible: false, reason: "extension" };
  }
  const minimum = Number(settings.minSizeMb || 0) * 1024 * 1024;
  if (!forcedTake && item.totalBytes > 0 && item.totalBytes < minimum) return { eligible: false, reason: "size" };

  return {
    eligible: true,
    url: HTTP_URL.test(item.finalUrl || "") ? item.finalUrl : item.url,
    fileName: basename(item.filename),
    headers: item.referrer ? { Referer: item.referrer } : {},
  };
}

/// 等待重定向链稳定（自适应，P1-8）。
///
/// 原实现固定 80/180/320ms 三轮，多级跳转 CDN（finalUrl 仍在变化）时会在
/// 链路中途取到临时中转 URL。现在：finalUrl 与上一轮相同且文件名就绪 → 提前
/// 结束；仍在变化 → 继续轮询，总预算约 2 秒封顶。
export async function refreshDownload(downloads, initial, wait = sleep) {
  let current = initial;
  let previousFinalUrl = current.finalUrl || "";
  for (const delay of [80, 180, 320, 420, 450, 450]) {
    await wait(delay);
    const [fresh] = await downloads.search({ id: initial.id });
    if (!fresh) break;
    current = fresh;
    const finalUrl = current.finalUrl || "";
    const stable = finalUrl && finalUrl === previousFinalUrl;
    previousFinalUrl = finalUrl;
    if (stable && current.filename) break;
  }
  return current;
}

// ---- 接管看门狗（P0-4）----
//
// MV3 Service Worker 随时可能被回收。如果 SW 在"已 pause、尚未 resume/cancel"
// 之间死亡，浏览器下载会永久停留在暂停状态。pause 前把下载 ID 记入
// storage.session（随浏览器会话存续，跨 SW 重启可读），退出接管流程时清除；
// background 的 alarms 看门狗定期调用 recoverStuckTakeovers 恢复超龄条目。
export const TAKEOVER_STUCK_MS = 90_000;

async function markPendingTakeover(id) {
  try {
    const session = chrome.storage?.session;
    if (!session?.get || !session?.set) return;
    const { pendingTakeover = {} } = await session.get("pendingTakeover");
    pendingTakeover[String(id)] = Date.now();
    await session.set({ pendingTakeover });
  } catch { /* 标记失败不阻断接管；看门狗只是兜底。 */ }
}

async function clearPendingTakeover(id) {
  try {
    const session = chrome.storage?.session;
    if (!session?.get || !session?.set) return;
    const { pendingTakeover = {} } = await session.get("pendingTakeover");
    if (!(String(id) in pendingTakeover)) return;
    delete pendingTakeover[String(id)];
    await session.set({ pendingTakeover });
  } catch {}
}

/// 恢复卡死的接管下载：条目超过 TAKEOVER_STUCK_MS 仍未清除时，
/// 若下载仍处于暂停态则 resume 放行，并移除标记。
/// "任务已发送但 cancel 失败"的下载会被主动清除标记（有意保持暂停避免
/// 产生重复文件），看门狗不会碰它们。
export async function recoverStuckTakeovers(deps = {}) {
  const downloads = deps.downloads || chrome.downloads;
  const session = deps.session || chrome.storage?.session;
  if (!downloads?.search || !session?.get || !session?.set) {
    return { recovered: [], remaining: {} };
  }
  let pending = {};
  try {
    pending = { ...((await session.get("pendingTakeover"))?.pendingTakeover || {}) };
  } catch {
    return { recovered: [], remaining: {} };
  }
  const now = deps.now || Date.now();
  const recovered = [];
  for (const [idText, timestamp] of Object.entries(pending)) {
    const id = Number(idText);
    if (!Number.isInteger(id) || now - Number(timestamp || 0) < TAKEOVER_STUCK_MS) continue;
    try {
      const [item] = await downloads.search({ id });
      if (item && item.paused) {
        await downloads.resume(id);
        recovered.push(id);
      }
    } catch { /* 下载可能已被接管完成并 erase；直接清除标记。 */ }
    delete pending[idText];
  }
  try { await session.set({ pendingTakeover: pending }); } catch {}
  return { recovered, remaining: pending };
}

/// 统一记录"被放行（未接管）"的下载（P2-14）。
///
/// 同时写两处：`lastIgnored`（最近一条，向后兼容）与 `ignoredList`
/// （环形缓冲，默认 20 条，popup 诊断区展示并支持一键改用猫步下载）。
async function recordIgnored(entry) {
  try {
    const { ignoredList = [] } = await chrome.storage.local.get("ignoredList");
    await chrome.storage.local.set({
      lastIgnored: entry,
      ignoredList: pushIgnoredEntry(ignoredList, entry),
    });
  } catch {}
}

// 通知节流：时间戳同时写入内存与 chrome.storage.session。
// MV3 Service Worker 空闲约 30 秒即被回收，纯内存节流在 SW 重启后失效，
// 会导致开机/唤醒期每个下载都重复弹通知；storage.session 随浏览器会话存续，
// 跨 SW 重启仍然有效。未配对提示使用更长周期，避免频繁打扰。
const NOTIFY_COOLDOWNS = {
  offline: 60_000,
  error: 60_000,
  unpaired: 5 * 60_000,
  "cancel-error": 60_000,
  "added": 60_000,
};
const memoryNotifyLast = {};

async function readPersistedNotifyLast(kind) {
  try {
    const stored = await chrome.storage.session.get(`notifyLast:${kind}`);
    return Number(stored?.[`notifyLast:${kind}`] || 0);
  } catch {
    return 0; // storage.session 不可用（旧浏览器/测试环境）时退化为纯内存节流。
  }
}

async function notifyThrottled(kind, title, message, notify, notificationId) {
  const now = Date.now();
  const cooldown = NOTIFY_COOLDOWNS[kind] ?? 60_000;
  if (now - (memoryNotifyLast[kind] || 0) <= cooldown) return false;
  memoryNotifyLast[kind] = now; // 先占位，收窄同一 SW 内并发请求的重复窗口。
  const persisted = await readPersistedNotifyLast(kind);
  if (now - persisted <= cooldown) return false;
  try { await chrome.storage.session.set({ [`notifyLast:${kind}`]: now }); } catch {}
  notify?.(title, message, notificationId);
  return true;
}

export function resetNotificationCooldownsForTest() {
  for (const kind of Object.keys(memoryNotifyLast)) delete memoryNotifyLast[kind];
  try {
    const session = chrome.storage?.session;
    if (session?.remove) {
      void session
        .remove(Object.keys(NOTIFY_COOLDOWNS).map((kind) => `notifyLast:${kind}`))
        .catch(() => {});
    }
  } catch {}
}

export async function interceptBrowserDownload(initial, options) {
  const { downloads, settings, runtimeId, sendTask, notify, wait, isDesktopOfflineError, swStartTime, onTakenOver } = options;
  const preflight = evaluateDownload(initial, settings, runtimeId, swStartTime);
  if (!preflight.eligible) {
    await recordIgnored({
      url: initial.url,
      filename: basename(initial.filename) || "未知文件",
      size: initial.totalBytes,
      reason: preflight.reason,
      timestamp: Date.now()
    });
    try { await downloads.resume(initial.id); } catch {}
    return false;
  }

  await markPendingTakeover(initial.id);
  try {
    await downloads.pause(initial.id);
  } catch {}

  let taskSent = false;
  try {
    const item = await refreshDownload(downloads, initial, wait);
    // 二次评估必须带 reevaluation 标记：此时下载已被本扩展 pause()，
    // paused/canResume/bytesReceived 不能再作为"历史任务"证据。
    const decision = evaluateDownload(item, settings, runtimeId, swStartTime, { reevaluation: true });
    if (!decision.eligible) {
      await recordIgnored({
        url: item.url,
        filename: basename(item.filename) || "未知文件",
        size: item.totalBytes,
        reason: decision.reason,
        timestamp: Date.now()
      });
      await downloads.resume(initial.id);
      await clearPendingTakeover(initial.id);
      return false;
    }
    const created = await sendTask(decision.url, decision.fileName, { headers: decision.headers });
    taskSent = true;
    await downloads.cancel(initial.id);
    await downloads.erase({ id: initial.id });
    await clearPendingTakeover(initial.id);
    // created（含任务 id）透传给徽章撤销（P3：接管成功后 5 秒内可取消）。
    onTakenOver?.(decision, created);
    return true;
  } catch (error) {
    if (!taskSent) {
      // SubTask 13.5：桌面端离线/无响应时明确通知用户，并确保浏览器原生下载继续。
      // 不静默失败；用户下载不丢失。
      const offline = isDesktopOfflineError ? isDesktopOfflineError(error) : isDefaultOfflineError(error);
      await recordIgnored({
        url: initial.url,
        filename: basename(initial.filename) || "未知文件",
        size: initial.totalBytes,
        reason: offline ? "offline" : `error:${error.message || String(error)}`,
        timestamp: Date.now()
      });
      try { await downloads.resume(initial.id); } catch { /* 下载可能已由浏览器结束。 */ }
      await clearPendingTakeover(initial.id);
      if (offline) {
        await notifyThrottled("offline", "桌面端离线，已回退浏览器下载", "点击此处打开猫步下载器", notify, "offline-warning");
      } else {
        await notifyThrottled("error", "接管失败，已回退浏览器下载", String(error?.message || error), notify, "takeover-error");
      }
      return false;
    }
    try { await downloads.cancel(initial.id); } catch { /* 保持暂停，避免产生重复文件。 */ }
    // 有意保持暂停（防重复文件）：主动清除标记，看门狗不接管此下载。
    await clearPendingTakeover(initial.id);
    await notifyThrottled("cancel-error", "任务已发送，但浏览器下载取消失败", "请在浏览器下载列表中手动取消重复任务", notify, "takeover-cancel-error");
    return true;
  }
}

// 默认离线判断（当 background.js 未注入 isDesktopOfflineError 时使用）。
// AbortError（fetch 超时中止）与"桌面端进程僵死"等同处理：走浏览器回退。
function isDefaultOfflineError(error) {
  const message = String(error?.message || error);
  return error instanceof TypeError
    || /Failed to fetch|NetworkError|fetch failed|ECONNREFUSED|connect ECONN|abort|timed?\s*out/i.test(message);
}

// 接管前的配对预检：未持有桌面端令牌时直接放行浏览器下载，
// 并按长周期（5 分钟）节流提示一次配对引导，避免未配对状态下
// 每个下载都经历"浮层 1.5 秒 → 发送失败 → 回退"并弹出重复错误通知。
// 返回 true 表示已按未配对处理（调用方应 resume 放行，不再进入接管流程）。
export async function skipUnpairedDownload(item, notify) {
  let bridgeToken;
  try {
    ({ bridgeToken } = await chrome.storage.local.get("bridgeToken"));
  } catch {
    return false; // 存储暂时不可读时不拦截下载，交由 sendTask 的错误路径兜底。
  }
  if (bridgeToken) return false;
  await recordIgnored({
    url: item.url,
    filename: basename(item.filename) || "未知文件",
    size: item.totalBytes,
    reason: "unpaired",
    timestamp: Date.now()
  });
  await notifyThrottled(
    "unpaired",
    "尚未与桌面端配对，本次由浏览器下载",
    "打开猫步下载器 → 设置 → 浏览器，输入配对码完成配对",
    notify,
    "unpaired-warning"
  );
  return true;
}

export { notifyThrottled };
