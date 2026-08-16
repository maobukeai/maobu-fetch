// SubTask 13.6：仅对语义元素做单元素探测。
// 严格遵循 AGENTS.md §5：禁止扫描全页所有链接（如 querySelectorAll('a') 全部抓取）。
// 仅查询 a[download]、video[src]、audio[src]、video source[src]、audio source[src]，
// 不再使用 Performance API 的全量资源扫描（历史实现已移除）。
const found = new Map();
// 上次发送给 background 的载荷（JSON）。内容未变化时不重复发送，避免无谓唤醒 SW。
let lastSentPayload = "";
// 单页跟踪的媒体条目上限：Map 按插入序淘汰最旧条目，防止长驻页面（SPA）无限增长。
const MAX_TRACKED_MEDIA = 100;

function collectMedia(send = true) {
  // 1. <a download href="...">：用户/页面显式标记的可下载链接。
  document.querySelectorAll("a[download]").forEach((node) => {
    const src = node.href;
    if (src && /^https?:/.test(src)) {
      found.set(src, { url: src, type: "download", title: document.title });
    }
  });
  // 2. <video src="..."> / <audio src="...">：直接带 src 的媒体元素。
  //    currentSrc 优先（覆盖 <source> 子元素解析后的最终地址）。
  document.querySelectorAll("video[src], audio[src]").forEach((node) => {
    const src = node.currentSrc || node.src;
    if (src && /^https?:/.test(src)) {
      found.set(src, { url: src, type: node.tagName.toLowerCase(), title: document.title });
    }
  });
  // 3. <video><source src="..."></video> / <audio><source src="..."></audio>：
  //    通过 source 子元素指定地址的媒体。仅查询 video/audio 内的 source，
  //    不查询孤立 <source>（无效元素）。
  document.querySelectorAll("video source[src], audio source[src]").forEach((node) => {
    const src = node.src;
    if (src && /^https?:/.test(src)) {
      found.set(src, { url: src, type: node.parentElement?.tagName.toLowerCase() || "media", title: document.title });
    }
  });
  while (found.size > MAX_TRACKED_MEDIA) {
    const oldest = found.keys().next().value;
    if (oldest === undefined) break;
    found.delete(oldest);
  }
  // 页内媒体悬浮下载按钮跟随最新探测结果显隐。
  syncFab();
  if (!send) return;
  // 无新增内容不发送；与上次相同也不发送。高频 DOM 页面（视频站、无限滚动）
  // 每次 mutation 都触发查询时，这里避免把 SW 反复从休眠中唤醒。
  const items = [...found.values()].slice(-20);
  if (!items.length) return;
  const payload = JSON.stringify(items);
  if (payload === lastSentPayload) return;
  lastSentPayload = payload;
  try {
    if (!isContextValid()) return;
    chrome.runtime.sendMessage({ type: "media", items }, () => {
      const _ = chrome?.runtime?.lastError;
    });
  } catch {}
}

function isContextValid() {
  try {
    return Boolean(chrome?.runtime?.id);
  } catch {
    return false;
  }
}

// ---- 页内媒体悬浮下载按钮 ----
// 检测到 <video>/<audio> 时在页面右下角显示"猫步下载"按钮，一键把媒体发送到桌面端。
// 两种发送模式：
// - direct：页面存在 http(s) 媒体直链（少数站点）时直接发送直链；
// - page：主流站点（B 站/YouTube/抖音等）使用 MSE，<video> 的 src 是 blob: 协议、
//   直链不可用，此时发送页面 URL，由桌面端走 yt-dlp 媒体分析流程。
// 使用 inline style 避免 CSP 限制；仅顶层框架注入（manifest 未开 all_frames）。
let fabUrl = "";
let fabMode = "direct";
let fabElement = null;
let fabStatusTimer = 0;
const FAB_LABEL = "⬇ 猫步下载";

function syncFab() {
  let hasMediaElement = false;
  try {
    hasMediaElement = Boolean(document.querySelector("video, audio"));
  } catch {}
  const direct = hasMediaElement
    ? [...found.values()].reverse().find((item) => item.type === "video" || item.type === "audio")
    : null;
  if (direct) {
    fabUrl = direct.url;
    fabMode = "direct";
  } else if (hasMediaElement) {
    fabUrl = location.href;
    fabMode = "page";
  } else {
    fabUrl = "";
  }
  if (!fabUrl || !isContextValid()) {
    if (fabElement) { fabElement.remove(); fabElement = null; }
    return;
  }
  if (!fabElement) {
    fabElement = createFab();
    (document.body || document.documentElement).appendChild(fabElement);
  }
}

function createFab() {
  const fab = document.createElement("div");
  fab.id = "maobu-fetch-media-fab";
  fab.setAttribute("role", "button");
  fab.setAttribute("tabindex", "0");
  fab.setAttribute("aria-label", "使用猫步下载器下载本页媒体");
  fab.title = "发送本页视频到猫步下载器";
  Object.assign(fab.style, {
    position: "fixed", bottom: "18px", right: "18px", zIndex: "2147483646",
    display: "inline-flex", alignItems: "center", gap: "6px",
    height: "34px", padding: "0 14px", borderRadius: "999px",
    background: "rgba(29, 29, 31, 0.92)", color: "#f5f5f7",
    fontSize: "12px", fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif",
    cursor: "pointer", userSelect: "none", WebkitUserSelect: "none",
    boxShadow: "0 6px 20px rgba(0, 0, 0, 0.25)",
    border: "1px solid rgba(255, 255, 255, 0.18)",
    transition: "transform 0.15s ease, opacity 0.15s ease",
    opacity: "0", transform: "translateY(6px)",
  });
  const label = document.createElement("span");
  label.textContent = FAB_LABEL;
  fab.appendChild(label);
  requestAnimationFrame(() => {
    if (!fabElement) return;
    fab.style.opacity = "1";
    fab.style.transform = "translateY(0)";
  });
  const sendToDesktop = () => {
    if (!fabUrl || !isContextValid()) return;
    label.textContent = "发送中…";
    // direct 模式走快速下载；page 模式走桌面端媒体分析（与右键"下载媒体"一致）。
    const payload = fabMode === "page"
      ? { type: "download-page-media", url: fabUrl, title: document.title }
      : { type: "send", url: fabUrl };
    try {
      chrome.runtime.sendMessage(payload, (response) => {
        const ok = Boolean(response?.ok);
        // 失败时完整错误已由 background 通过系统通知展示，此处截断避免胶囊过宽。
        const errorText = String(response?.error || "发送失败");
        showFabStatus(label, ok
          ? "✓ 已发送"
          : `✕ ${errorText.length > 40 ? `${errorText.slice(0, 40)}…` : errorText}`);
      });
    } catch {
      showFabStatus(label, "✕ 发送失败");
    }
  };
  fab.onclick = sendToDesktop;
  fab.onkeydown = (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      sendToDesktop();
    }
  };
  return fab;
}

function showFabStatus(label, text) {
  label.textContent = text;
  clearTimeout(fabStatusTimer);
  fabStatusTimer = setTimeout(() => { label.textContent = FAB_LABEL; }, 2500);
}

if (isContextValid()) {
  collectMedia();
  // 高频 DOM 变化（动画、无限滚动、聊天流）合并为至多每 800ms 一次查询，
  // 避免每次 mutation 都执行 querySelectorAll 并唤醒 Service Worker。
  let mutationTimer = 0;
  const observer = new MutationObserver(() => {
    if (!isContextValid()) {
      observer.disconnect();
      return;
    }
    if (!mutationTimer) {
      mutationTimer = setTimeout(() => {
        mutationTimer = 0;
        collectMedia(true);
      }, 800);
    }
  });
  if (document.documentElement) {
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["src", "href"],
    });
  }
  const timer = setInterval(() => {
    if (!isContextValid()) {
      clearInterval(timer);
      return;
    }
    collectMedia(true);
  }, 5000);
}

// SubTask 13.4：接管前 1.5 秒浮层。
// background 在 interceptBrowserDownload 之前发送 show-overlay 消息；
// content script 显示浮层，用户点击"本次绕过"返回 { bypass: true }，
// 1.5 秒超时返回 { bypass: false }（即自动接管）。
// 浮层使用 inline style 注入，避免触发页面 CSP；z-index 设为最大值确保置顶。
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message.type !== "show-overlay") return false;
  let resolved = false;
  const overlay = createOverlay(message.fileName || "", () => {
    if (resolved) return;
    resolved = true;
    sendResponse({ bypass: true });
  });
  (document.body || document.documentElement).appendChild(overlay);
  setTimeout(() => {
    if (resolved) return;
    resolved = true;
    sendResponse({ bypass: false });
    overlay.remove();
  }, 1500);
  // 返回 true 保持 sendResponse 通道打开（异步响应）。
  return true;
});

function createOverlay(fileName, onBypass) {
  const overlay = document.createElement("div");
  overlay.id = "maobu-fetch-takeover-overlay";
  overlay.setAttribute("data-maobu", "1");
  // 使用 inline style 避免 CSP 阻止 <style> 标签；position:fixed 确保不破坏页面布局。
  Object.assign(overlay.style, {
    position: "fixed",
    top: "16px",
    right: "16px",
    zIndex: "2147483647",
    maxWidth: "320px",
    minWidth: "250px",
    padding: "12px 14px",
    borderRadius: "10px",
    background: "rgba(246, 246, 248, 0.88)",
    backdropFilter: "blur(12px) saturate(180%)",
    webkitBackdropFilter: "blur(12px) saturate(180%)",
    border: "1px solid rgba(255, 255, 255, 0.6)",
    boxShadow: "0 10px 30px rgba(0, 0, 0, 0.12), 0 2px 6px rgba(0, 0, 0, 0.05)",
    color: "#1d1d1f",
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif",
    fontSize: "12px",
    lineHeight: "1.4",
    display: "flex",
    flexDirection: "column",
    gap: "8px",
    transition: "opacity 0.2s, transform 0.2s",
    opacity: "0",
    transform: "translateY(-6px)",
  });
  // 强制下一帧设置 opacity，触发 transition。
  requestAnimationFrame(() => {
    overlay.style.opacity = "1";
    overlay.style.transform = "translateY(0)";
  });

  const title = document.createElement("div");
  title.textContent = "将被猫步下载器接管";
  title.style.fontWeight = "600";
  title.style.color = "#1d1d1f";
  title.style.fontSize = "13px";
  overlay.appendChild(title);

  const subtitle = document.createElement("div");
  subtitle.textContent = truncate(fileName, 60) || "本次下载将转交桌面端处理";
  subtitle.style.color = "#6e6e73";
  subtitle.style.fontSize = "11px";
  subtitle.style.overflow = "hidden";
  subtitle.style.textOverflow = "ellipsis";
  subtitle.style.whiteSpace = "nowrap";
  overlay.appendChild(subtitle);

  const buttonRow = document.createElement("div");
  buttonRow.style.display = "flex";
  buttonRow.style.justifyContent = "flex-end";
  buttonRow.style.gap = "8px";

  const bypassBtn = document.createElement("button");
  bypassBtn.textContent = "本次绕过";
  Object.assign(bypassBtn.style, {
    padding: "4px 12px",
    borderRadius: "6px",
    border: "1px solid rgba(0, 0, 0, 0.12)",
    background: "rgba(0, 0, 0, 0.05)",
    color: "#1d1d1f",
    fontSize: "11px",
    fontWeight: "500",
    cursor: "pointer",
    transition: "background-color 0.15s",
  });
  bypassBtn.onmouseenter = () => { bypassBtn.style.background = "rgba(0, 0, 0, 0.12)"; };
  bypassBtn.onmouseleave = () => { bypassBtn.style.background = "rgba(0, 0, 0, 0.05)"; };
  bypassBtn.onclick = () => {
    overlay.remove();
    onBypass();
  };
  buttonRow.appendChild(bypassBtn);
  overlay.appendChild(buttonRow);

  return overlay;
}

function truncate(value, max) {
  const text = String(value || "");
  return text.length > max ? text.slice(0, max - 1) + "…" : text;
}
