import { sendWithCurrentPageAuth, buildCookieHeader } from "./auth-download.js";

const $ = (id) => document.getElementById(id);
const [tab] = await chrome.tabs.query({ active: true, currentWindow: true }).catch(() => [null]);
const stored = await chrome.storage.local.get(["intercept", "minSizeMb", "bypassUntil"]).catch(() => ({}));

const interceptEl = $("intercept");
if (interceptEl) interceptEl.checked = stored.intercept ?? true;
const minSizeEl = $("minSize");
if (minSizeEl) minSizeEl.value = String(stored.minSizeMb ?? 1);

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
  const pairBoxEl = $("pairBox");
  if (pairBoxEl) pairBoxEl.classList.toggle("hidden", !online || paired);
  message(!online ? "请先启动猫步下载器；下载会保留在浏览器中" : paired ? "连接安全，可以发送下载" : "需要先在下方输入 6 位配对码完成授权", !online || !paired);
}
await health().catch(() => {});
const refreshEl = $("refresh");
if (refreshEl) refreshEl.onclick = async () => {
  await health().catch(() => {});
  await renderTasks().catch(() => {});
};

const pairEl = $("pair");
if (pairEl) {
  pairEl.onclick = async () => {
    const codeEl = $("pairCode");
    if (!codeEl) return;
    const code = codeEl.value.trim();
    if (!/^\d{6}$/.test(code)) return message("请输入 6 位配对码", true);
    const result = await call({ type: "pair", code });
    if (result?.ok) {
      message("配对成功");
      await health().catch(() => {});
    } else {
      message(`配对失败：${result?.error || "未知错误"}`, true);
    }
  };
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
      const name = decodeURIComponent(item.url.split("/").pop()?.split("?")[0] || item.title || "媒体资源").slice(0, 80);
      row.innerHTML = "<i>↓</i><div><b></b><small></small></div><button title='发送'>＋</button>";
      row.querySelector("b").textContent = name;
      row.querySelector("small").textContent = item.type;
      row.querySelector("button").onclick = () => send(item.url, name);
      mediaEl.append(row);
    });
  }
}

async function send(url, fileName) {
  if (!url) return;
  message("正在发送…");
  const response = await call({ type: "send", url, fileName });
  message(response?.ok ? "已发送到桌面端" : `发送失败：${response?.error || "请检查配对状态"}`, !response?.ok);
}

const sendEl = $("send");
const urlEl = $("url");
if (sendEl && urlEl) {
  sendEl.onclick = () => send(urlEl.value.trim());
  urlEl.onkeydown = (event) => {
    if (event.key === "Enter") sendEl.click();
  };
}

// SubTask 45.1～45.5：使用当前页面登录态下载。
// 仅在当前 tab 是 http(s) 页面时显示按钮；点击后调用 auth-download.js 中的
// sendWithCurrentPageAuth 辅助函数：从 chrome.cookies.getAll 获取当前页 Cookie，
// 通过本地桥 /v1/tasks/add 一次性传递给桌面端，不写入扩展 storage。
// 不上传浏览历史或页面数据，仅传递当前页 Cookie（AGENTS.md §5）。
const authDownloadSection = $("authDownload");
const useAuthDownloadEl = $("useAuthDownload");
const currentTabUrl = tab?.url || "";
const isHttpTab = /^https?:/i.test(currentTabUrl);
if (authDownloadSection && isHttpTab) {
  authDownloadSection.classList.remove("hidden");
}

// Automatically sync cookies to the desktop app when the popup opens on a supported domain
if (isHttpTab) {
  (async () => {
    const domainsToSync = ["douyin.com", "tiktok.com", "bilibili.com", "weibo.com", "weibo.cn", "youtube.com", "twitter.com", "x.com"];
    try {
      const urlObj = new URL(currentTabUrl);
      const hostname = urlObj.hostname.toLowerCase();
      const matchedDomain = domainsToSync.find(d => hostname === d || hostname.endsWith("." + d));
      if (matchedDomain) {
        let baseDomain = matchedDomain;
        if (baseDomain === "weibo.cn") baseDomain = "weibo.com";
        if (baseDomain === "x.com") baseDomain = "twitter.com";
        const cookies = await chrome.cookies.getAll({ url: currentTabUrl }).catch(() => []);
        const cookieHeader = buildCookieHeader(cookies);
        if (cookieHeader) {
          await call({ type: "sync-cookies", domain: baseDomain, cookie: cookieHeader }).catch(() => {});
        }
      }
    } catch {}
  })();
}
if (useAuthDownloadEl) {
  useAuthDownloadEl.onclick = async () => {
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

if (interceptEl) {
  interceptEl.onchange = async (event) => {
    await chrome.storage.local.set({ intercept: event.target.checked }).catch(() => {});
    await renderDiag().catch(() => {});
  };
}
if (minSizeEl) {
  minSizeEl.onchange = async (event) => {
    await chrome.storage.local.set({ minSizeMb: Number(event.target.value) }).catch(() => {});
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
  await renderDiag().catch(() => {});
};

const bypassEl = $("bypass");
if (bypassEl) bypassEl.onclick = toggleBypass;
const topBypassEl = $("topBypass");
if (topBypassEl) topBypassEl.onclick = toggleBypass;

async function renderDiag() {
  const diagBox = $("diagBox");
  if (!diagBox) return;
  const data = await chrome.storage.local.get(["lastIgnored", "minSizeMb"]).catch(() => ({}));
  const diag = data.lastIgnored;
  if (diag) {
    diagBox.classList.remove("hidden");
    const diagFile = $("diagFile");
    if (diagFile) {
      diagFile.textContent = diag.filename || "未知文件";
      diagFile.title = diag.filename || "";
    }
    const diagSize = $("diagSize");
    if (diagSize) {
      diagSize.textContent = diag.size > 0 ? `${(diag.size / (1024 * 1024)).toFixed(2)} MB` : "未知";
    }
    const reasonMap = {
      disabled: "「接管浏览器下载」开关未开启",
      bypass: "处于临时暂停接管时段",
      self: "扩展本身发起的下载（防循环限制）",
      scheme: "链接非 HTTP/HTTPS 协议（如 blob/data/file 协议）",
      extension: "文件后缀名不在接管后缀名规则列表内",
      size: `文件体积小于设置的接管大小（当前设为了 ${data.minSizeMb ?? 1} MB）`,
    };
    let readable = reasonMap[diag.reason] || diag.reason || "未知原因";
    if (diag.reason && typeof diag.reason === "string" && diag.reason.startsWith("error:")) {
      readable = `桌面桥接连接失败（${diag.reason.slice(6)}），已自动回退到浏览器默认下载`;
    }
    const diagReason = $("diagReason");
    if (diagReason) diagReason.textContent = readable;
  } else {
    diagBox.classList.add("hidden");
  }
}
await renderDiag().catch(() => {});

const clearDiagEl = $("clearDiag");
if (clearDiagEl) {
  clearDiagEl.onclick = async () => {
    await chrome.storage.local.remove("lastIgnored").catch(() => {});
    await renderDiag().catch(() => {});
  }
}

// SubTask 13.3：最近桌面端任务列表。
// 从 background 拉取 /v1/tasks/recent，渲染文件名、状态图标、进度条、速度，
// 提供 暂停/继续/打开文件 操作按钮。桌面端离线时显示明确提示。
const STATUS_ICON = {
  queued: "⏳", downloading: "↓", paused: "⏸", completed: "✓", failed: "✕",
  cancelled: "–", scheduled: "🕖", verifying: "✓", interrupted: "!",
  "waiting-network": "!", "remote-changed": "↻", "paused-by-low-disk": "⏸",
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
  return `${(bytesPerSec / 1024 / 1024).toFixed(2)} MB/s`;
}

async function renderTasks() {
  const tasksEl = $("tasks");
  const countEl = $("taskCount");
  if (!tasksEl) return;
  const response = await call({ type: "recent-tasks" });
  if (!response?.ok) {
    tasksEl.innerHTML = '<div class="empty">桌面端离线或未配对</div>';
    if (countEl) countEl.textContent = "0";
    return;
  }
  const tasks = response.result?.tasks || [];
  if (countEl) countEl.textContent = String(tasks.length);
  if (!tasks.length) {
    tasksEl.innerHTML = '<div class="empty">暂无桌面端任务</div>';
    return;
  }
  tasksEl.innerHTML = "";
  for (const task of tasks) {
    const row = document.createElement("div");
    row.className = "task-item";
    const icon = STATUS_ICON[task.status] || "•";
    const label = STATUS_LABEL[task.status] || task.status;
    const progress = Math.round((task.progress || 0) * 100);
    const canPause = PAUSABLE.has(task.status);
    const canResume = RESUMABLE.has(task.status);
    const canOpen = task.status === "completed";
    const actionBtn = canPause
      ? `<button class="pause" title="暂停" data-action="pause">⏸</button>`
      : canResume
      ? `<button class="resume" title="继续" data-action="resume">▶</button>`
      : `<button class="pause" title="不可暂停" disabled>⏸</button>`;
    const openBtn = canOpen
      ? `<button class="open" title="打开文件" data-action="open_file">📂</button>`
      : `<button class="open" title="未完成" disabled>📂</button>`;
    row.innerHTML = `<i class="${task.status}" title="${label}">${icon}</i>`
      + `<div class="task-info">`
      + `<b class="task-name" title=""></b>`
      + `<div class="task-bar ${task.status}"><span style="width:${progress}%"></span></div>`
      + `<div class="task-meta"><span>${label}</span><span>·</span><span class="task-speed">${formatSpeed(task.speed)}</span><span>·</span><span>${progress}%</span></div>`
      + (task.error ? `<div class="task-error"></div>` : "")
      + `</div>`
      + `<div class="task-actions">${actionBtn}${openBtn}</div>`;
    row.querySelector(".task-name").textContent = task.file_name || task.url || "未知文件";
    row.querySelector(".task-name").title = task.file_name || task.url || "";
    if (task.error) {
      const errEl = row.querySelector(".task-error");
      if (errEl) errEl.textContent = task.error;
    }
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
    tasksEl.append(row);
  }
}

await renderTasks().catch(() => {
  const tasksEl = $("tasks");
  if (tasksEl) tasksEl.innerHTML = '<div class="empty">桌面端离线或未配对</div>';
});

// 弹窗开启时，按 1 秒间隔轮询更新最新任务状态、下载进度与速度
setInterval(() => {
  void renderTasks().catch(() => {});
}, 1000);


