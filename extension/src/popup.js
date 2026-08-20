import { sendWithCurrentPageAuth, buildCookieHeader, exportCurrentPageCookies } from "./auth-download.js";
import { describeIgnoredReason, shortReasonLabel } from "./reasons.js";
import { matchMediaDomain } from "./domains.js";

const $ = (id) => document.getElementById(id);
const [tab] = await chrome.tabs.query({ active: true, currentWindow: true }).catch(() => [null]);
const stored = await chrome.storage.local.get([
  "intercept", "minSizeMb", "bypassUntil", "takeoverMode", "interceptMagnet", "mediaQuality",
  "autoDelayMs", "subtitlePref",
]).catch(() => ({}));

const interceptEl = $("intercept");
if (interceptEl) interceptEl.checked = stored.intercept ?? true;

// ---- 最小文件大小：预设下拉 + 自定义输入（P3-21） ----
const MIN_SIZE_PRESETS = [0, 1, 5, 20, 100];
const minSizeEl = $("minSize");
const minSizeCustomEl = $("minSizeCustom");
function syncMinSizeUi(value) {
  if (!minSizeEl || !minSizeCustomEl) return;
  const numeric = Number(value ?? 1);
  if (MIN_SIZE_PRESETS.includes(numeric)) {
    minSizeEl.value = String(numeric);
    minSizeCustomEl.classList.add("hidden");
    minSizeCustomEl.value = "";
  } else {
    minSizeEl.value = "custom";
    minSizeCustomEl.classList.remove("hidden");
    minSizeCustomEl.value = String(Math.max(1, Math.floor(numeric || 1)));
  }
}
syncMinSizeUi(stored.minSizeMb ?? 1);

const takeoverModeEl = $("takeoverMode");
if (takeoverModeEl) takeoverModeEl.value = stored.takeoverMode === "ask" ? "ask" : "auto";
// 自动接管倒计时（0–5000ms，默认 1500）。不在预设中的值就近吸附到最近的档位。
const autoDelayEl = $("autoDelay");
const AUTO_DELAY_PRESETS = [0, 1000, 1500, 2000, 3000, 5000];
if (autoDelayEl) {
  const value = Number(stored.autoDelayMs ?? 1500);
  const nearest = AUTO_DELAY_PRESETS.reduce((best, preset) =>
    Math.abs(preset - value) < Math.abs(best - value) ? preset : best, 1500);
  autoDelayEl.value = String(nearest);
}
const interceptMagnetEl = $("interceptMagnet");
if (interceptMagnetEl) interceptMagnetEl.checked = stored.interceptMagnet ?? true;
const mediaQualityEl = $("mediaQuality");
if (mediaQualityEl) mediaQualityEl.value = ["best", "1080", "720", "audio"].includes(stored.mediaQuality) ? stored.mediaQuality : "best";
const subtitlePrefEl = $("subtitlePref");
if (subtitlePrefEl) subtitlePrefEl.value = ["all", "zh", "none"].includes(stored.subtitlePref) ? stored.subtitlePref : "all";

function message(text, error = false) {
  const el = $("message");
  if (el) {
    el.textContent = text;
    el.classList.toggle("error", error);
  }
}
function call(payload) {
  return new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage(payload, resolve);
    } catch {
      resolve(null);
    }
  });
}

async function health() {
  const response = await call({ type: "health" });
  const online = Boolean(response?.ok);
  const paired = Boolean(response?.paired);
  const desktopVersion = String(response?.version || "");
  const extVersion = chrome.runtime.getManifest()?.version || "";
  const statusEl = $("status");
  if (statusEl) {
    statusEl.classList.toggle("online", online && paired);
    statusEl.classList.toggle("unpaired", online && !paired);
  }
  const connEl = $("connection");
  if (connEl) {
    if (!online) connEl.textContent = "桌面端未连接";
    else if (!paired) connEl.textContent = "桌面端在线（未配对）";
    else connEl.textContent = "桌面端已连接";
  }
  // 版本一致性提示：扩展与桌面端版本同步发布，不一致时引导更新扩展并重载。
  const updateBoxEl = $("updateBox");
  const updateTextEl = $("updateText");
  if (updateBoxEl && updateTextEl) {
    const mismatch = online && desktopVersion && extVersion && desktopVersion !== extVersion;
    updateBoxEl.classList.toggle("hidden", !mismatch);
    if (mismatch) {
      updateTextEl.textContent = `桌面端为 v${desktopVersion}，扩展为 v${extVersion}。请在桌面端「设置 → 关于」一键更新扩展后，点击下方按钮重载。`;
    }
  }
  const pairBoxEl = $("pairBox");
  if (pairBoxEl) pairBoxEl.classList.toggle("hidden", !online || paired);
  // 配对框可见时自动聚焦输入框：打开弹窗即可直接键入/粘贴配对码。
  if (online && !paired) $("pairCode")?.focus();
  message(!online ? "请先启动猫步下载器；下载会保留在浏览器中" : paired ? "连接安全，可以发送下载" : "需要先在下方输入 6 位配对码完成授权", !online || !paired);
}
await health().catch(() => {});
const reloadExtEl = $("reloadExt");
if (reloadExtEl) reloadExtEl.onclick = () => chrome.runtime.reload();
const openGithubEl = $("openGithub");
if (openGithubEl) {
  openGithubEl.onclick = () => {
    chrome.tabs.create({ url: "https://github.com/maobukeai/maobu-fetch" });
  };
}
const refreshEl = $("refresh");
if (refreshEl) refreshEl.onclick = async () => {
  await health().catch(() => {});
  await renderTasks().catch(() => {});
};

// ---- 配对（P2-12：粘贴自动去非数字、满 6 位自动提交） ----
async function doPair() {
  const codeEl = $("pairCode");
  if (!codeEl) return;
  const code = codeEl.value.replace(/\D/g, "").slice(0, 6);
  codeEl.value = code;
  if (!/^\d{6}$/.test(code)) return message("请输入 6 位配对码", true);
  const pairBtn = $("pair");
  if (pairBtn) pairBtn.disabled = true;
  try {
    const result = await call({ type: "pair", code });
    if (result?.ok) {
      message("配对成功");
      await health().catch(() => {});
    } else {
      message(`配对失败：${result?.error || "未知错误"}`, true);
    }
  } finally {
    if (pairBtn) pairBtn.disabled = false;
  }
}
const pairEl = $("pair");
if (pairEl) pairEl.onclick = doPair;
const pairCodeEl = $("pairCode");
if (pairCodeEl) {
  pairCodeEl.addEventListener("input", () => {
    const digits = pairCodeEl.value.replace(/\D/g, "").slice(0, 6);
    if (pairCodeEl.value !== digits) pairCodeEl.value = digits;
    if (digits.length === 6) void doPair();
  });
  pairCodeEl.addEventListener("keydown", (event) => {
    if (event.key === "Enter") void doPair();
  });
}

const tabId = tab?.id;
const key = tabId ? `media:${tabId}` : "";
let items = [];
if (key && chrome.storage.session) {
  try {
    const session = await chrome.storage.session.get(key);
    items = session[key] || [];
  } catch {}
}
const countEl = $("count");
if (countEl) countEl.textContent = String(items.length);

if (items.length) {
  const mediaEl = $("media");
  if (mediaEl) {
    mediaEl.innerHTML = "";
    items.slice(-10).reverse().forEach((item) => {
      const row = document.createElement("div");
      row.className = "media-item";
      // URL 可能含非法 % 序列，decodeURIComponent 会抛 URIError 导致整区渲染失败。
      let name = item.title || "媒体资源";
      try {
        name = decodeURIComponent(item.url.split("/").pop()?.split("?")[0] || "") || name;
      } catch { /* 保留原始段 */ }
      name = String(name).slice(0, 80);
      row.innerHTML = "<i>↓</i><div><b></b><small></small></div><button title='发送'>＋</button>";
      row.querySelector("b").textContent = name;
      row.querySelector("small").textContent = item.type;
      // 媒体 URL 来自当前页面元素，附带页面 Referer 提升鉴权 CDN 的成功率。
      row.querySelector("button").onclick = () => send(item.url, name, { headers: { Referer: currentTabUrl } });
      mediaEl.append(row);
    });
  }
}

async function send(url, fileName, extra) {
  if (!url) return;
  message("正在发送…");
  const response = await call({ type: "send", url, fileName, extra });
  message(response?.ok ? "已发送到桌面端" : `发送失败：${response?.error || "请检查配对状态"}`, !response?.ok);
}

const sendEl = $("send");
const urlEl = $("url");
// 磁力徽标 + 剪贴板粘贴（P3）：识别 magnet: 时显示 🧲，粘贴按钮一键填入。
const magnetBadgeEl = $("magnetBadge");
const pasteEl = $("paste");
const syncMagnetBadge = () => {
  if (!magnetBadgeEl || !urlEl) return;
  magnetBadgeEl.classList.toggle("hidden", !/^magnet:/i.test(urlEl.value.trim()));
};
if (urlEl) urlEl.addEventListener("input", syncMagnetBadge);
if (pasteEl) {
  pasteEl.onclick = async () => {
    try {
      const text = String(await navigator.clipboard.readText() || "").trim();
      if (!text) { message("剪贴板为空", true); return; }
      if (urlEl) { urlEl.value = text; syncMagnetBadge(); urlEl.focus(); }
    } catch {
      message("无法读取剪贴板，请手动粘贴", true);
    }
  };
}
if (sendEl && urlEl) {
  sendEl.onclick = () => send(urlEl.value.trim());
  urlEl.onkeydown = (event) => {
    if (event.key === "Enter") sendEl.click();
  };
}

// SubTask 45.1～45.5：使用当前页面登录态下载。
// 仅在当前 tab 是 http(s) 页面时显示按钮；点击后调用 auth-download.js 中的
// sendWithCurrentPageAuth 辅助函数：从 chrome.cookies.getAll 获取当前页 Cookie，
// 通过本地桥一次性传递给桌面端，不写入扩展 storage。
const authDownloadSection = $("authDownload");
const useAuthDownloadEl = $("useAuthDownload");
const currentTabUrl = tab?.url || "";
const isHttpTab = /^https?:/i.test(currentTabUrl);
if (authDownloadSection && isHttpTab) {
  authDownloadSection.classList.remove("hidden");
}

// 打开弹窗时同步当前媒体平台 Cookie（域名表统一来自 domains.js，
// 含用户在选项页配置的自定义同步域名，与 background 的逻辑保持一致）。
if (isHttpTab) {
  (async () => {
    try {
      const urlObj = new URL(currentTabUrl);
      const { customMediaDomains = [] } = await chrome.storage.local.get("customMediaDomains").catch(() => ({}));
      const baseDomain = matchMediaDomain(urlObj.hostname, customMediaDomains);
      if (!baseDomain) return;
      // 显式传入 tab.cookieStoreId 以支持无痕窗口（无痕 Cookie 在独立 store 中）
      const getAllParams = { url: currentTabUrl };
      if (tab?.cookieStoreId) getAllParams.storeId = tab.cookieStoreId;
      let cookies = await chrome.cookies.getAll(getAllParams).catch(() => []);

      if (baseDomain) {
        const extraDomains = [baseDomain, baseDomain.replace(/^(?:pan|drive|www)\./i, "")];
        for (const d of extraDomains) {
          if (!d) continue;
          const subCookies = await chrome.cookies.getAll({
            domain: d.startsWith(".") ? d : `.${d}`,
            ...(tab?.cookieStoreId ? { storeId: tab.cookieStoreId } : {})
          }).catch(() => []);
          const map = new Map(cookies.map(c => [c.name, c]));
          for (const sc of subCookies) {
            if (sc?.name && !map.has(sc.name)) {
              map.set(sc.name, sc);
            }
          }
          cookies = Array.from(map.values());
        }
      }

      const cookieHeader = buildCookieHeader(cookies);
      if (cookieHeader) {
        await call({ type: "sync-cookies", domain: baseDomain, cookie: cookieHeader }).catch(() => {});
      }
    } catch {}
  })();
}
if (useAuthDownloadEl) {
  const authMenuEl = $("authMenu");
  const authOneShotEl = $("authOneShot");
  const authExportCookiesEl = $("authExportCookies");

  const closeAuthMenu = () => {
    if (authMenuEl) authMenuEl.classList.add("hidden");
    useAuthDownloadEl.setAttribute("aria-expanded", "false");
  };
  const toggleAuthMenu = () => {
    if (!authMenuEl) return;
    const willOpen = authMenuEl.classList.contains("hidden");
    authMenuEl.classList.toggle("hidden", !willOpen);
    useAuthDownloadEl.setAttribute("aria-expanded", willOpen ? "true" : "false");
  };

  useAuthDownloadEl.onclick = () => {
    if (!isHttpTab) {
      message("当前页面不是 HTTP/HTTPS 页面，无法获取登录态", true);
      return;
    }
    toggleAuthMenu();
  };

  // 点击下拉外部收起菜单
  document.addEventListener("click", (event) => {
    if (authMenuEl?.classList.contains("hidden")) return;
    const dropdown = $("authDropdown");
    if (dropdown && !dropdown.contains(event.target)) closeAuthMenu();
  });
  // ESC 收起
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") closeAuthMenu();
  });

  if (authOneShotEl) {
    authOneShotEl.onclick = async () => {
      closeAuthMenu();
      if (!isHttpTab) {
        message("当前页面不是 HTTP/HTTPS 页面，无法获取登录态", true);
        return;
      }
      if (!useAuthDownloadEl.disabled) useAuthDownloadEl.disabled = true;
      try {
        // 下载 URL 优先使用快速下载输入框的值（如有），否则使用当前页 URL。
        const downloadUrl = urlEl?.value?.trim() || currentTabUrl;
        message("正在发送登录态…");
        const result = await sendWithCurrentPageAuth({
          url: downloadUrl,
          pageUrl: currentTabUrl,
          userAgent: navigator.userAgent,
          cookiesApi: chrome.cookies,
          sendMessage: call,
          cookieStoreId: tab?.cookieStoreId,
        });
        if (result.ok) {
          message("已发送登录态到桌面端");
        } else {
          message(`登录态发送失败：${result.error}`, true);
        }
      } catch (error) {
        message(`登录态发送失败：${error?.message || error}`, true);
      } finally {
        useAuthDownloadEl.disabled = false;
      }
    };
  }

  if (authExportCookiesEl) {
    authExportCookiesEl.onclick = async () => {
      closeAuthMenu();
      if (!isHttpTab) {
        message("当前页面不是 HTTP/HTTPS 页面，无法获取登录态", true);
        return;
      }
      if (!authExportCookiesEl.disabled) authExportCookiesEl.disabled = true;
      try {
        message("正在导出 cookies.txt…");
        const triggerDownload = (content, fileName) => new Promise((resolve, reject) => {
          try {
            const blob = new Blob([content], { type: "text/plain" });
            const url = URL.createObjectURL(blob);
            const a = document.createElement("a");
            a.href = url;
            a.download = fileName;
            document.body.appendChild(a);
            a.click();
            setTimeout(() => {
              document.body.removeChild(a);
              URL.revokeObjectURL(url);
              resolve();
            }, 100);
          } catch (e) {
            reject(e);
          }
        });
        const result = await exportCurrentPageCookies({
          pageUrl: currentTabUrl,
          cookiesApi: chrome.cookies,
          cookieStoreId: tab?.cookieStoreId,
          triggerDownload,
        });
        if (result.ok) {
          message(`已导出 ${result.fileName}，请在「设置 → 媒体凭证」中导入`);
        } else {
          message(`导出失败：${result.error}`, true);
        }
      } catch (error) {
        message(`导出失败：${error?.message || error}`, true);
      } finally {
        authExportCookiesEl.disabled = false;
      }
    };
  }
}

// ---- 设置项 ----
if (interceptEl) {
  interceptEl.onchange = async (event) => {
    await chrome.storage.local.set({ intercept: event.target.checked }).catch(() => {});
  };
}
if (takeoverModeEl) {
  takeoverModeEl.onchange = async (event) => {
    await chrome.storage.local.set({ takeoverMode: event.target.value === "ask" ? "ask" : "auto" }).catch(() => {});
  };
}
if (autoDelayEl) {
  autoDelayEl.onchange = async (event) => {
    await chrome.storage.local.set({ autoDelayMs: Number(event.target.value) || 0 }).catch(() => {});
  };
}
if (subtitlePrefEl) {
  subtitlePrefEl.onchange = async (event) => {
    await chrome.storage.local.set({ subtitlePref: event.target.value }).catch(() => {});
  };
}
if (interceptMagnetEl) {
  interceptMagnetEl.onchange = async (event) => {
    await chrome.storage.local.set({ interceptMagnet: event.target.checked }).catch(() => {});
  };
}
if (mediaQualityEl) {
  mediaQualityEl.onchange = async (event) => {
    await chrome.storage.local.set({ mediaQuality: event.target.value }).catch(() => {});
  };
}
if (minSizeEl) {
  minSizeEl.onchange = async (event) => {
    if (event.target.value === "custom") {
      minSizeCustomEl?.classList.remove("hidden");
      minSizeCustomEl?.focus();
      return;
    }
    minSizeCustomEl?.classList.add("hidden");
    await chrome.storage.local.set({ minSizeMb: Number(event.target.value) }).catch(() => {});
    await renderDiag().catch(() => {});
  };
}
if (minSizeCustomEl) {
  minSizeCustomEl.onchange = async (event) => {
    const value = Math.max(1, Math.floor(Number(event.target.value) || 0));
    event.target.value = String(value);
    await chrome.storage.local.set({ minSizeMb: value }).catch(() => {});
    await renderDiag().catch(() => {});
  };
}

const editRulesEl = $("editRules");
if (editRulesEl) {
  editRulesEl.onclick = () => {
    try {
      chrome.tabs.create({ url: chrome.runtime.getURL("options.html") });
    } catch {}
  };
}

// ---- 当前站点快捷决策（P2-10） ----
const siteMatches = (hostname, key) => hostname === key || hostname.endsWith(`.${key}`);
const currentHost = (() => {
  try { return new URL(currentTabUrl).hostname.toLowerCase(); } catch { return ""; }
})();

function resolveSiteChoice(siteChoices, hostname) {
  for (const [key, value] of Object.entries(siteChoices || {})) {
    if ((value === "take" || value === "bypass") && siteMatches(hostname, key)) return value;
  }
  return null;
}

async function renderSite() {
  const box = $("siteBox");
  if (!box || !currentHost) return;
  box.classList.remove("hidden");
  const hostLabel = $("siteHostLabel");
  if (hostLabel) hostLabel.textContent = currentHost;
  const { siteChoices = {}, fabHiddenHosts = [] } = await chrome.storage.local
    .get(["siteChoices", "fabHiddenHosts"]).catch(() => ({}));
  const choice = resolveSiteChoice(siteChoices, currentHost);
  const badge = $("siteChoiceBadge");
  if (badge) {
    badge.classList.toggle("hidden", !choice);
    badge.textContent = choice === "take" ? "总是接管" : "总是放行";
    badge.classList.toggle("take", choice === "take");
    badge.classList.toggle("bypass", choice === "bypass");
  }
  const clearEl = $("siteClearChoice");
  if (clearEl) clearEl.classList.toggle("hidden", !choice);
  const restoreEl = $("restoreFab");
  if (restoreEl) {
    restoreEl.classList.toggle("hidden", !fabHiddenHosts.some((h) => siteMatches(currentHost, h)));
  }
}

async function writeSiteChoice(value) {
  try {
    const { siteChoices = {} } = await chrome.storage.local.get("siteChoices");
    if (value) siteChoices[currentHost] = value;
    else for (const key of Object.keys(siteChoices)) {
      if (siteMatches(currentHost, key)) delete siteChoices[key];
    }
    await chrome.storage.local.set({ siteChoices });
  } catch {}
  await renderSite().catch(() => {});
}

const siteAlwaysTakeEl = $("siteAlwaysTake");
if (siteAlwaysTakeEl) siteAlwaysTakeEl.onclick = () => { void writeSiteChoice("take"); message("已记住：此站点总是接管"); };
const siteAlwaysBypassEl = $("siteAlwaysBypass");
if (siteAlwaysBypassEl) siteAlwaysBypassEl.onclick = () => { void writeSiteChoice("bypass"); message("已记住：此站点由浏览器下载"); };
const grabPageResourcesEl = $("grabPageResources");
if (grabPageResourcesEl) grabPageResourcesEl.onclick = async () => {
  try {
    await call({ type: "grab-page-resources", tabId });
    window.close();
  } catch (e) {
    message("抓取失败: " + String(e));
  }
};
const siteClearChoiceEl = $("siteClearChoice");
if (siteClearChoiceEl) siteClearChoiceEl.onclick = () => { void writeSiteChoice(null); message("已清除站点记忆"); };
const restoreFabEl = $("restoreFab");
if (restoreFabEl) restoreFabEl.onclick = async () => {
  try {
    const { fabHiddenHosts = [] } = await chrome.storage.local.get("fabHiddenHosts");
    await chrome.storage.local.set({
      fabHiddenHosts: fabHiddenHosts.filter((h) => !siteMatches(currentHost, h)),
    });
    message("已恢复此站点的悬浮下载按钮");
  } catch {}
  await renderSite().catch(() => {});
};
await renderSite().catch(() => {});

// ---- 流嗅探（按站点开关，仅内存，AGENTS.md §5）----
// 列表按 URL 签名差量渲染：2 秒刷新与用户点击不竞争
// （全量重建会替换掉正要点击的按钮，与任务列表 P1-9 同理）。
let lastSniffSignature = "";
async function renderSniffer() {
  const toggleEl = $("sniffToggle");
  const boxEl = $("sniffBox");
  const listEl = $("sniffList");
  if (!toggleEl || !currentHost || !tabId) return;
  toggleEl.classList.remove("hidden");
  const response = await call({ type: "sniffed-media", tabId, host: currentHost }).catch(() => null);
  const enabled = Boolean(response?.enabled);
  const items = Array.isArray(response?.items) ? response.items : [];
  toggleEl.textContent = enabled ? "流嗅探：开" : "流嗅探：关";
  toggleEl.classList.toggle("active", enabled);
  if (!boxEl || !listEl) return;
  const signature = JSON.stringify([enabled, items.map((item) => item.url)]);
  if (!enabled || !items.length) {
    boxEl.classList.add("hidden");
    lastSniffSignature = "";
    return;
  }
  boxEl.classList.remove("hidden");
  const countEl = $("sniffCount");
  if (countEl) countEl.textContent = String(items.length);
  if (signature === lastSniffSignature) return; // 内容未变，保留现有 DOM
  lastSniffSignature = signature;
  listEl.innerHTML = "";
  for (const item of items.slice(0, 8)) {
    const row = document.createElement("div");
    row.className = "media-item";
    const kindIcon = item.kind === "stream" ? "📶" : item.kind === "video" ? "🎬" : item.kind === "audio" ? "🎵" : "⬇";
    const name = (item.name || item.url || "媒体资源").slice(0, 80);
    // stream 类（m3u8/mpd/ts 分片）不能直连下载（只会得到播放列表文本或片段），
    // 改走桌面端媒体解析（yt-dlp 原生支持 HLS/DASH）。
    const isStream = item.kind === "stream";
    row.innerHTML = `<i>${kindIcon}</i><div><b></b><small></small></div>`
      + `<button title='${isStream ? "通过桌面端媒体解析下载（HLS 流需要 yt-dlp）" : "发送"}'>${isStream ? "⚡" : "＋"}</button>`;
    row.querySelector("b").textContent = name;
    row.querySelector("b").title = item.url || "";
    row.querySelector("small").textContent = isStream ? "流媒体（解析下载）" : item.kind || "";
    row.querySelector("button").onclick = () => {
      if (!isStream) return send(item.url, name, { headers: { Referer: currentTabUrl } });
      message("正在解析流媒体…（可能需要安装 yt-dlp）");
      void call({ type: "download-page-media", url: item.url, title: name }).then((response) => {
        message(response?.ok ? "已发送到桌面端解析下载" : `解析失败：${response?.error || "请检查桌面端连接"}`, !response?.ok);
      });
    };
    listEl.append(row);
  }
}
const sniffToggleEl = $("sniffToggle");
if (sniffToggleEl) {
  sniffToggleEl.onclick = async () => {
    const response = await call({ type: "sniffed-media", tabId, host: currentHost }).catch(() => null);
    await call({ type: "sniff-toggle", host: currentHost, enabled: !response?.enabled }).catch(() => {});
    message(response?.enabled ? "已关闭此站点的流嗅探" : "已开启此站点的流嗅探（仅记录媒体直链，不上传）");
    await renderSniffer().catch(() => {});
  };
}
await renderSniffer().catch(() => {});

// ---- 临时绕过 ----
async function updateBypassButton() {
  const bypassEl = $("bypass");
  const topBypassEl = $("topBypass");
  const { bypassUntil } = await chrome.storage.local.get("bypassUntil").catch(() => ({}));
  const remainingMs = Number(bypassUntil || 0) - Date.now();
  if (remainingMs > 0) {
    const remainingMins = Math.ceil(remainingMs / 60_000);
    if (bypassEl) {
      bypassEl.textContent = `恢复接管（接管已暂停，剩余 ${remainingMins} 分钟）`;
      bypassEl.classList.add("active");
    }
    if (topBypassEl) {
      topBypassEl.textContent = `▶ 恢复接管 (${remainingMins}m)`;
      topBypassEl.classList.add("active");
      topBypassEl.title = `接管已暂停，剩余 ${remainingMins} 分钟。点击恢复接管`;
    }
  } else {
    if (bypassEl) {
      bypassEl.textContent = "暂停接管 10 分钟";
      bypassEl.classList.remove("active");
    }
    if (topBypassEl) {
      topBypassEl.textContent = "⏸ 暂停接管 10m";
      topBypassEl.classList.remove("active");
      topBypassEl.title = "点击临时暂停接管 10 分钟";
    }
  }
}
await updateBypassButton().catch(() => {});

const toggleBypass = async () => {
  const { bypassUntil } = await chrome.storage.local.get("bypassUntil").catch(() => ({}));
  const isActive = Number(bypassUntil || 0) > Date.now();
  if (isActive) {
    await call({ type: "bypass", cancel: true });
    message("已取消绕过，恢复下载接管");
  } else {
    await call({ type: "bypass", minutes: 10 });
    message("接管已暂停 10 分钟");
  }
  await updateBypassButton().catch(() => {});
};

const bypassEl = $("bypass");
if (bypassEl) bypassEl.onclick = toggleBypass;
const topBypassEl = $("topBypass");
if (topBypassEl) topBypassEl.onclick = toggleBypass;

// ---- 放行历史（P2-14：环形缓冲 + 一键改用猫步下载） ----
function formatClock(timestamp) {
  const date = new Date(Number(timestamp || 0));
  if (isNaN(date.getTime())) return "";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

async function renderDiag() {
  const diagBox = $("diagBox");
  const listEl = $("ignoredList");
  if (!diagBox || !listEl) return;
  const data = await chrome.storage.local.get(["ignoredList", "lastIgnored", "minSizeMb"]).catch(() => ({}));
  const entries = Array.isArray(data.ignoredList) && data.ignoredList.length
    ? data.ignoredList
    : (data.lastIgnored ? [data.lastIgnored] : []);
  if (!entries.length) {
    diagBox.classList.add("hidden");
    return;
  }
  diagBox.classList.remove("hidden");
  // 按原因分组统计（最多展示 3 种，帮助用户一眼看出"为什么没被接管"）。
  const summaryEl = $("diagSummary");
  if (summaryEl) {
    const counts = new Map();
    for (const entry of entries) {
      const label = shortReasonLabel(entry.reason);
      counts.set(label, (counts.get(label) || 0) + 1);
    }
    const parts = [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 3)
      .map(([label, count]) => `${label} ×${count}`);
    const rest = counts.size - parts.length;
    summaryEl.textContent = `原因分布：${parts.join(" · ")}${rest > 0 ? ` · 另 ${rest} 种` : ""}`;
  }
  listEl.innerHTML = "";
  for (const entry of entries.slice(-10).reverse()) {
    const row = document.createElement("div");
    row.className = "ignored-item";
    const info = document.createElement("div");
    info.className = "ignored-info";
    const name = document.createElement("b");
    name.textContent = entry.filename || "未知文件";
    name.title = entry.url || "";
    const meta = document.createElement("small");
    const sizeText = entry.size > 0 ? `${(entry.size / (1024 * 1024)).toFixed(2)} MB` : "未知大小";
    meta.textContent = `${describeIgnoredReason(entry.reason, data.minSizeMb)} · ${sizeText}${formatClock(entry.timestamp) ? ` · ${formatClock(entry.timestamp)}` : ""}`;
    info.append(name, meta);
    row.append(info);
    if (/^https?:/i.test(entry.url || "")) {
      const resend = document.createElement("button");
      resend.textContent = "猫步下载";
      resend.title = "把这条下载改用猫步下载器重新下载";
      resend.onclick = () => send(entry.url, entry.filename);
      row.append(resend);
    }
    listEl.append(row);
  }
}
await renderDiag().catch(() => {});

const clearDiagEl = $("clearDiag");
if (clearDiagEl) {
  clearDiagEl.onclick = async () => {
    await chrome.storage.local.remove(["lastIgnored", "ignoredList"]).catch(() => {});
    await renderDiag().catch(() => {});
  };
}

// ---- 最近桌面端任务（SubTask 13.3 + P1-9 差量渲染 + P3-22 大小/ETA/重试） ----
const STATUS_ICON = {
  queued: "⏳", downloading: "↓", paused: "⏸", completed: "✓", failed: "✕", cancelled: "–",
  scheduled: "🕖", verifying: "✓", interrupted: "!", "waiting-network": "!", "remote-changed": "↻", "paused-by-low-disk": "⏸",
};
const STATUS_LABEL = {
  queued: "等待中", downloading: "下载中", paused: "已暂停", completed: "已完成",
  failed: "失败", cancelled: "已取消", scheduled: "已计划", verifying: "校验中",
  interrupted: "已中断", "waiting-network": "等待网络", "remote-changed": "远端变化",
  "paused-by-low-disk": "磁盘不足",
};
const PAUSABLE = new Set(["downloading", "queued", "scheduled", "verifying"]);
const RESUMABLE = new Set(["paused", "failed", "cancelled", "interrupted", "waiting-network", "remote-changed", "paused-by-low-disk"]);

function formatSpeed(bytesPerSec) {
  if (!bytesPerSec || bytesPerSec <= 0) return "—";
  if (bytesPerSec < 1024) return `${bytesPerSec} B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
  if (bytesPerSec < 1024 * 1024 * 1024) return `${(bytesPerSec / 1024 / 1024).toFixed(2)} MB/s`;
  return `${(bytesPerSec / 1024 / 1024 / 1024).toFixed(2)} GB/s`;
}

function formatSize(totalBytes) {
  const bytes = Number(totalBytes || 0);
  if (!bytes || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatEta(task) {
  if (task.status !== "downloading" || !task.total_bytes || !task.speed) return "";
  const remainingSec = Math.round((task.total_bytes * (1 - (task.progress || 0))) / task.speed);
  if (remainingSec < 1) return "";
  if (remainingSec >= 3600) return `剩余 ${Math.floor(remainingSec / 3600)} 时 ${Math.floor((remainingSec % 3600) / 60)} 分`;
  return `剩余 ${Math.floor(remainingSec / 60)}:${String(remainingSec % 60).padStart(2, "0")}`;
}

function buildTaskRow(task) {
  const row = document.createElement("div");
  row.className = "task-item";
  row.dataset.taskId = task.id;
  const label = STATUS_LABEL[task.status] || task.status;
  const canPause = PAUSABLE.has(task.status);
  const isFailed = task.status === "failed";
  const canResume = RESUMABLE.has(task.status);
  const canOpen = task.status === "completed";
  const actionBtn = canPause
    ? `<button class="pause" title="暂停" data-action="pause">⏸</button>`
    : canResume
    ? `<button class="resume" title="${isFailed ? "重试" : "继续"}" data-action="resume">${isFailed ? "⟳" : "▶"}</button>`
    : `<button class="pause" title="不可暂停" disabled>⏸</button>`;
  const openBtn = canOpen
    ? `<button class="open" title="打开文件" data-action="open_file">📂</button>`
    : `<button class="open" title="未完成" disabled>📂</button>`;
  row.innerHTML = `<i class="${task.status}" title="${label}">${STATUS_ICON[task.status] || "•"}</i>`
    + `<div class="task-info">`
    + `<b class="task-name" title=""></b>`
    + `<div class="task-bar ${task.status}"><span style="width:0%"></span></div>`
    + `<div class="task-meta"><span class="task-label">${label}</span><span>·</span><span class="task-speed"></span><span class="task-size-wrap">· <span class="task-size"></span></span><span class="task-eta-wrap">· <span class="task-eta"></span></span><span>·</span><span class="task-pct"></span></div>`
    + `<div class="task-error"></div>`
    + `</div>`
    + `<div class="task-actions">${actionBtn}${openBtn}</div>`;
  const nameEl = row.querySelector(".task-name");
  nameEl.textContent = task.file_name || task.url || "未知文件";
  nameEl.title = task.file_name || task.url || "";
  row.querySelectorAll("button[data-action]").forEach((btn) => {
    btn.onclick = async (event) => {
      const action = event.currentTarget.dataset.action;
      btn.disabled = true;
      const result = await call({ type: "task-action", id: task.id, action });
      if (!result?.ok || !result.result?.success) {
        message(`操作失败：${result?.error || result?.result?.error || "未知错误"}`, true);
      } else {
        message(action === "open_file" ? "已请求打开文件" : action === "pause" ? "已暂停" : "已继续");
      }
      await renderTasks().catch(() => {});
    };
  });
  updateTaskRow(row, task);
  return row;
}

function updateTaskRow(row, task) {
  const progress = Math.round((task.progress || 0) * 100);
  row.querySelector(".task-bar > span").style.width = `${progress}%`;
  row.querySelector(".task-speed").textContent = formatSpeed(task.speed);
  row.querySelector(".task-pct").textContent = `${progress}%`;
  const sizeEl = row.querySelector(".task-size");
  const sizeWrap = row.querySelector(".task-size-wrap");
  const sizeText = formatSize(task.total_bytes);
  if (sizeEl) sizeEl.textContent = sizeText;
  if (sizeWrap) sizeWrap.style.display = sizeText ? "" : "none";
  const etaEl = row.querySelector(".task-eta");
  const etaWrap = row.querySelector(".task-eta-wrap");
  const etaText = formatEta(task);
  if (etaEl) etaEl.textContent = etaText;
  if (etaWrap) etaWrap.style.display = etaText ? "" : "none";
  const errEl = row.querySelector(".task-error");
  if (errEl) {
    errEl.textContent = task.error || "";
    errEl.style.display = task.error ? "" : "none";
  }
}

// 差量渲染（P1-9）：同一任务状态不变时只更新进度/速度/ETA 文本节点，
// 不重建 DOM——此前每秒全量重建会替换掉用户正要点击的按钮。
const taskRows = new Map();

async function renderTasks() {
  const tasksEl = $("tasks");
  const countEl2 = $("taskCount");
  if (!tasksEl) return;
  const response = await call({ type: "recent-tasks" });
  if (!response?.ok) {
    // 桌面端错误文本可能含 URL/尖括号，用 textContent 渲染避免注入。
    const errReason = response?.error || "桌面端离线或未配对";
    tasksEl.innerHTML = "";
    const emptyRow = document.createElement("div");
    emptyRow.className = "empty";
    emptyRow.textContent = errReason;
    tasksEl.appendChild(emptyRow);
    if (countEl2) countEl2.textContent = "0";
    taskRows.clear();
    updateTaskSummary([]);
    return;
  }
  const tasks = response.result?.tasks || [];
  if (countEl2) countEl2.textContent = String(tasks.length);
  updateTaskSummary(tasks);
  if (!tasks.length) {
    tasksEl.innerHTML = '<div class="empty">暂无桌面端任务</div>';
    taskRows.clear();
    return;
  }
  const emptyEl = tasksEl.querySelector(".empty");
  if (emptyEl) emptyEl.remove();
  const activeIds = new Set(tasks.map((task) => task.id));
  for (const [id, entry] of taskRows) {
    if (!activeIds.has(id)) {
      entry.row.remove();
      taskRows.delete(id);
    }
  }
  for (const task of tasks) {
    const existing = taskRows.get(task.id);
    if (existing && existing.status === task.status) {
      updateTaskRow(existing.row, task);
    } else {
      // 状态变化时重建行，必须同步移除旧行——否则同任务出现重复行
      //（如 downloading → paused → completed 的长任务生命周期）。
      if (existing) existing.row.remove();
      const row = buildTaskRow(task);
      taskRows.set(task.id, { row, status: task.status });
    }
  }
  // 按最新顺序重排（appendChild 会移动既有节点，不触发重建）。
  for (const task of tasks) {
    tasksEl.appendChild(taskRows.get(task.id).row);
  }
}

// ---- 活动任务聚合条与批量重试（长任务管理）----
const ACTIVE_STATUSES = new Set(["downloading", "queued", "verifying", "scheduled"]);

function updateTaskSummary(tasks) {
  const rowEl = $("taskSummaryRow");
  const summaryEl = $("taskSummary");
  const retryEl = $("retryFailed");
  if (!rowEl || !summaryEl) return;
  const active = tasks.filter((task) => ACTIVE_STATUSES.has(task.status));
  const failed = tasks.filter((task) => task.status === "failed");
  const totalSpeed = tasks.reduce((sum, task) => sum + (task.status === "downloading" ? Number(task.speed || 0) : 0), 0);
  const summaryText = active.length
    ? `${active.length} 个进行中${totalSpeed > 0 ? ` · 合计 ${formatSpeed(totalSpeed)}` : ""}`
    : "";
  summaryEl.textContent = summaryText;
  if (retryEl) {
    retryEl.classList.toggle("hidden", failed.length === 0);
    retryEl.textContent = `重试失败（${failed.length}）`;
  }
  rowEl.classList.toggle("hidden", !summaryText && failed.length === 0);
}

const retryFailedEl = $("retryFailed");
if (retryFailedEl) {
  retryFailedEl.onclick = async () => {
    retryFailedEl.disabled = true;
    try {
      const response = await call({ type: "recent-tasks" }).catch(() => null);
      const failedTasks = (response?.result?.tasks || []).filter((task) => task.status === "failed");
      let retried = 0;
      for (const task of failedTasks) {
        const result = await call({ type: "task-action", id: task.id, action: "resume" });
        if (result?.ok && result.result?.success) retried += 1;
      }
      message(retried
        ? `已重新开始 ${retried} 个失败任务`
        : `没有任务被重试${failedTasks.length ? "（桌面端拒绝了操作）" : ""}`, retried === 0 && failedTasks.length > 0);
      await renderTasks().catch(() => {});
    } finally {
      retryFailedEl.disabled = false;
    }
  };
}

await renderTasks().catch(() => {
  const tasksEl = $("tasks");
  if (tasksEl) tasksEl.innerHTML = '<div class="empty">桌面端离线或未配对</div>';
});

// 智能轮询：有活动任务时 1 秒刷新进度；空闲时降到 3 秒，减少无意义的桥接请求。
// （AGENTS §8：仅弹窗打开期间轮询，桌面端仍为事件驱动增量更新，非全量高频轮询。）
let pollTimer = 0;
let pollIntervalMs = 0;
function scheduleTaskPoll(intervalMs) {
  if (pollIntervalMs === intervalMs) return;
  pollIntervalMs = intervalMs;
  clearInterval(pollTimer);
  pollTimer = setInterval(() => { void pollTasks(); }, intervalMs);
}
async function pollTasks() {
  await renderTasks().catch(() => {});
  const activeCount = [...taskRows.values()]
    .filter((entry) => ACTIVE_STATUSES.has(entry.status)).length;
  scheduleTaskPoll(activeCount > 0 ? 1000 : 3000);
}
void pollTasks();

// 弹窗打开期间低频刷新嗅探列表（页面播放器可能持续产生新的媒体请求）。
setInterval(() => {
  if (document.visibilityState === "visible") void renderSniffer().catch(() => {});
}, 2000);
