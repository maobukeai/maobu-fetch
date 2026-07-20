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

export function evaluateDownload(item, settings, runtimeId) {
  if (!settings.intercept) return { eligible: false, reason: "disabled" };
  if (Date.now() < Number(settings.bypassUntil || 0)) return { eligible: false, reason: "bypass" };
  if (item.byExtensionId === runtimeId) return { eligible: false, reason: "self" };

  const urls = [...new Set([item.finalUrl, item.url].filter((url) => HTTP_URL.test(url || "")))];
  if (!urls.length) return { eligible: false, reason: "scheme" };
  const hosts = urls.map(host).filter(Boolean);
  if (hosts.some((hostname) => matchesHost(hostname, settings.blockHosts || []))) {
    return { eligible: false, reason: "blocked-host" };
  }
  if ((settings.allowHosts || []).length && !hosts.some((hostname) => matchesHost(hostname, settings.allowHosts))) {
    return { eligible: false, reason: "not-allowed-host" };
  }

  const extension = extensionFrom(item, urls);
  if ((settings.extensions || []).length && !settings.extensions.includes(extension)) {
    return { eligible: false, reason: "extension" };
  }
  const minimum = Number(settings.minSizeMb || 0) * 1024 * 1024;
  if (item.totalBytes > 0 && item.totalBytes < minimum) return { eligible: false, reason: "size" };

  return {
    eligible: true,
    url: HTTP_URL.test(item.finalUrl || "") ? item.finalUrl : item.url,
    fileName: basename(item.filename),
    headers: item.referrer ? { Referer: item.referrer } : {},
  };
}

export async function refreshDownload(downloads, initial, wait = sleep) {
  let current = initial;
  let previousFinalUrl = current.finalUrl || "";
  for (const delay of [80, 180, 320]) {
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

export async function interceptBrowserDownload(initial, options) {
  const { downloads, settings, runtimeId, sendTask, notify, wait, isDesktopOfflineError } = options;
  const preflight = evaluateDownload(initial, settings, runtimeId);
  if (!preflight.eligible) {
    try {
      await chrome.storage.local.set({
        lastIgnored: {
          url: initial.url,
          filename: basename(initial.filename) || "未知文件",
          size: initial.totalBytes,
          reason: preflight.reason,
          timestamp: Date.now()
        }
      });
    } catch {}
    try { await downloads.resume(initial.id); } catch {}
    return false;
  }

  try {
    await downloads.pause(initial.id);
  } catch {}

  let taskSent = false;
  try {
    const item = await refreshDownload(downloads, initial, wait);
    const decision = evaluateDownload(item, settings, runtimeId);
    if (!decision.eligible) {
      try {
        await chrome.storage.local.set({
          lastIgnored: {
            url: item.url,
            filename: basename(item.filename) || "未知文件",
            size: item.totalBytes,
            reason: decision.reason,
            timestamp: Date.now()
          }
        });
      } catch {}
      await downloads.resume(initial.id);
      return false;
    }
    await sendTask(decision.url, decision.fileName, { headers: decision.headers });
    taskSent = true;
    await downloads.cancel(initial.id);
    await downloads.erase({ id: initial.id });
    return true;
  } catch (error) {
    if (!taskSent) {
      // SubTask 13.5：桌面端离线时明确通知用户，并确保浏览器原生下载继续。
      // 不静默失败；用户下载不丢失。
      const offline = isDesktopOfflineError ? isDesktopOfflineError(error) : isDefaultOfflineError(error);
      try {
        await chrome.storage.local.set({
          lastIgnored: {
            url: initial.url,
            filename: basename(initial.filename) || "未知文件",
            size: initial.totalBytes,
            reason: offline ? "offline" : `error:${error.message || String(error)}`,
            timestamp: Date.now()
          }
        });
      } catch {}
      try { await downloads.resume(initial.id); } catch { /* 下载可能已由浏览器结束。 */ }
      if (offline) {
        notify?.("桌面端离线，已回退浏览器下载", "请启动猫步下载器后重试");
      } else {
        notify?.("接管失败，已回退浏览器下载", String(error?.message || error));
      }
      return false;
    }
    try { await downloads.cancel(initial.id); } catch { /* 保持暂停，避免产生重复文件。 */ }
    notify?.("任务已发送，但浏览器下载取消失败", "请在浏览器下载列表中手动取消重复任务");
    return true;
  }
}

// 默认离线判断（当 background.js 未注入 isDesktopOfflineError 时使用）。
function isDefaultOfflineError(error) {
  const message = String(error?.message || error);
  return error instanceof TypeError
    || /Failed to fetch|NetworkError|fetch failed|ECONNREFUSED|connect ECONN/i.test(message);
}
