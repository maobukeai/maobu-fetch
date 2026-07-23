// SubTask 13.6：仅对语义元素做单元素探测。
// 严格遵循 AGENTS.md §5：禁止扫描全页所有链接（如 querySelectorAll('a') 全部抓取）。
// 仅查询 a[download]、video[src]、audio[src]、video source[src]、audio source[src]，
// 不再使用 Performance API 的全量资源扫描（历史实现已移除）。
const found = new Map();

function collectMedia() {
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
  try {
    if (!isContextValid()) return;
    chrome.runtime.sendMessage({ type: "media", items: [...found.values()].slice(-20) }, () => {
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

if (isContextValid()) {
  collectMedia();
  const observer = new MutationObserver(() => {
    if (!isContextValid()) {
      observer.disconnect();
      return;
    }
    collectMedia();
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
    collectMedia();
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
