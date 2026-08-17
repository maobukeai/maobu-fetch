// 猫步下载器内容脚本（经典脚本，非 ES module——manifest content_scripts 不支持 module）。
//
// 职责：
//   1. 语义元素媒体探测（SubTask 13.6，禁止全页链接扫描）
//   2. 流嗅探缓存：接收 background 推送的媒体直链（仅用户开启嗅探的站点）
//   3. 页内媒体悬浮下载按钮（FAB）：可拖动吸附边缘、双击复位、右键菜单、
//      可按站点隐藏、page 模式显示分析状态
//   4. magnet: 链接点击拦截（BT-08）
//   5. 消息入口：接管浮层 / 成功徽章（含撤销）转发到 content-ui.js 的 UI 组件
//
// 浮层/徽章/轻提示/FAB 菜单等 UI 组件定义在 content-ui.js（先于本文件注入）。
// 使用 inline style 注入，避免触发页面 CSP；仅顶层框架（manifest 未开 all_frames）。
(() => {
  // P0-2：防重复注入守卫。background 在消息不可达时会用
  // chrome.scripting.executeScript 重新注入 content-ui.js + 本文件；没有守卫时
  // 观察器、定时器、FAB 与消息监听都会注册两份（双倍内存、双 FAB、重复消息）。
  // 扩展重载后的新隔离世界不携带旧标记，会正常重新初始化。
  if (window.__maobuFetchContentInjected) return;
  window.__maobuFetchContentInjected = true;

  const Ui = window.MaobuUi;
  const FONT_STACK = "-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif";
  const isContextValid = () => {
    try { return Boolean(chrome?.runtime?.id); } catch { return false; }
  };
  const truncate = (value, max) => {
    const text = String(value || "");
    return text.length > max ? text.slice(0, max - 1) + "…" : text;
  };
  const sendMessageSafe = (payload) => new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage(payload, (response) => {
        const _ = chrome?.runtime?.lastError; // 吞掉上下文失效时的报错
        resolve(response || null);
      });
    } catch { resolve(null); }
  });
  const showBadge = (kind, text, options) => { try { Ui?.showBadge?.(kind, text, options); } catch {} };

  // ==================== 媒体探测（SubTask 13.6） ====================
  // 仅对语义元素做单元素探测：a[download]、video/audio 的 src 与 source 子元素。
  // 严格遵循 AGENTS.md §5：禁止扫描全页所有链接。
  const found = new Map();
  let lastSentPayload = "";
  const MAX_TRACKED_MEDIA = 100;

  function collectMedia(send = true) {
    document.querySelectorAll("a[download]").forEach((node) => {
      const src = node.href;
      if (src && /^https?:/.test(src)) {
        found.set(src, { url: src, type: "download", title: document.title });
      }
    });
    document.querySelectorAll("video[src], audio[src]").forEach((node) => {
      const src = node.currentSrc || node.src;
      if (src && /^https?:/.test(src)) {
        found.set(src, { url: src, type: node.tagName.toLowerCase(), title: document.title });
      }
    });
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
    syncFab();
    if (!send) return;
    // 无新增内容不发送；与上次相同也不发送。高频 DOM 页面（视频站、无限滚动）
    // 每次 mutation 都触发查询时，这里避免把 SW 反复从休眠中唤醒。
    const items = [...found.values()].slice(-20);
    if (!items.length) return;
    const payload = JSON.stringify(items);
    if (payload === lastSentPayload) return;
    lastSentPayload = payload;
    if (!isContextValid()) return;
    void sendMessageSafe({ type: "media", items });
  }

  // ==================== 流嗅探缓存（仅用户开启嗅探的站点才收到推送） ====================
  // background 的 webRequest 观察器把页面播放器发出的媒体直链（m3u8/mp4 等）
  // 推送到这里。FAB 只对 video/audio 直连（HTTP 内核可直接下载）；stream 类
  // （m3u8/ts 分片）直连只会下载到播放列表文本或几秒片段，退回 page 模式
  // 走桌面端 yt-dlp 解析。
  let sniffedItems = [];
  function latestSniffedTarget() {
    for (const kind of ["video", "audio"]) {
      for (let i = sniffedItems.length - 1; i >= 0; i -= 1) {
        if (sniffedItems[i]?.kind === kind) return sniffedItems[i].url;
      }
    }
    return "";
  }

  // ==================== 页内悬浮下载按钮（FAB） ====================
  let fabUrl = "";
  let fabMode = "direct";
  let fabElement = null;
  let fabLabel = null;
  let fabStatusTimer = 0;
  let fabHiddenHosts = [];
  let fabPos = null; // { side: "left"|"right", offset, bottom }（吸附边缘后）
  const FAB_LABEL = "⬇ 猫步下载";

  // 站点隐藏列表与位置缓存：popup 的"恢复悬浮按钮"会改写存储，
  // 通过 onChanged 保持本页缓存同步。
  (async () => {
    try {
      const stored = await chrome.storage.local.get(["fabHiddenHosts", "fabPos"]);
      fabHiddenHosts = Array.isArray(stored.fabHiddenHosts) ? stored.fabHiddenHosts : [];
      fabPos = stored.fabPos || null;
      syncFab();
    } catch {}
  })();
  try {
    chrome.storage.onChanged.addListener((changes, area) => {
      if (area !== "local") return;
      if (changes.fabHiddenHosts) {
        fabHiddenHosts = Array.isArray(changes.fabHiddenHosts.newValue) ? changes.fabHiddenHosts.newValue : [];
        syncFab();
      }
      if (changes.fabPos) fabPos = changes.fabPos.newValue || null;
    });
  } catch {}

  const isFabHiddenHere = () => {
    const hostname = location.hostname.toLowerCase();
    return fabHiddenHosts.some((host) => hostname === host || hostname.endsWith(`.${host}`));
  };

  function syncFab() {
    let hasMediaElement = false;
    try {
      hasMediaElement = Boolean(document.querySelector("video, audio"));
    } catch {}
    const sniffed = latestSniffedTarget();
    const direct = sniffed
      || (hasMediaElement ? [...found.values()].reverse().find((item) => item.type === "video" || item.type === "audio") : null);
    if (direct) {
      fabUrl = direct;
      fabMode = "direct";
    } else if (hasMediaElement) {
      fabUrl = location.href;
      fabMode = "page";
    } else {
      fabUrl = "";
    }
    if (!fabUrl || !isContextValid() || isFabHiddenHere()) {
      if (fabElement) { fabElement.remove(); fabElement = null; }
      return;
    }
    if (!fabElement) {
      fabElement = createFab();
      (document.body || document.documentElement).appendChild(fabElement);
    }
  }

  function applyFabPosition(fab) {
    const pos = fabPos;
    if (!pos) return; // 默认位置（右下 18px）由初始 style 提供
    const maxBottom = Math.max(8, window.innerHeight - 60);
    fab.style.bottom = `${Math.min(Number(pos.bottom || 18), maxBottom)}px`;
    if (pos.side === "left") {
      fab.style.left = `${Number(pos.offset || 18)}px`;
      fab.style.right = "auto";
    } else {
      fab.style.right = `${Number(pos.offset || 18)}px`;
      fab.style.left = "auto";
    }
  }

  function createFab() {
    const fab = document.createElement("div");
    fab.id = "maobu-fetch-media-fab";
    fab.setAttribute("role", "button");
    fab.setAttribute("tabindex", "0");
    fab.setAttribute("aria-label", "使用猫步下载器下载本页媒体");
    fab.title = "发送本页视频到猫步下载器（右键：菜单；可拖动；双击复位）";
    Object.assign(fab.style, {
      position: "fixed", bottom: "18px", right: "18px", zIndex: "2147483646",
      display: "inline-flex", alignItems: "center", gap: "6px",
      height: "34px", padding: "0 14px", borderRadius: "999px",
      background: "rgba(29, 29, 31, 0.92)", color: "#f5f5f7",
      fontSize: "12px", fontFamily: FONT_STACK,
      cursor: "pointer", userSelect: "none", WebkitUserSelect: "none", touchAction: "none",
      boxShadow: "0 6px 20px rgba(0, 0, 0, 0.25)",
      border: "1px solid rgba(255, 255, 255, 0.18)",
      transition: "transform 0.15s ease, opacity 0.15s ease",
      opacity: "0", transform: "translateY(6px)",
    });
    const label = document.createElement("span");
    label.textContent = FAB_LABEL;
    fab.appendChild(label);
    fabLabel = label;
    applyFabPosition(fab);
    requestAnimationFrame(() => {
      if (!fabElement) return;
      fab.style.opacity = "1";
      fab.style.transform = "translateY(0)";
    });

    // ---- 拖动 vs 点击：位移超过 6px 视为拖动，松手后吸附最近边缘 ----
    let dragState = null;
    fab.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      dragState = {
        startX: event.clientX, startY: event.clientY, moved: false,
        width: fab.offsetWidth, height: fab.offsetHeight,
      };
      try { fab.setPointerCapture(event.pointerId); } catch {}
    });
    fab.addEventListener("pointermove", (event) => {
      if (!dragState) return;
      const dx = event.clientX - dragState.startX;
      const dy = event.clientY - dragState.startY;
      if (!dragState.moved && Math.hypot(dx, dy) < 6) return;
      dragState.moved = true;
      const right = Math.max(8, Math.min(window.innerWidth - event.clientX - dragState.width / 2, window.innerWidth - 60));
      const bottom = Math.max(8, Math.min(window.innerHeight - event.clientY - dragState.height / 2, window.innerHeight - 60));
      fab.style.right = `${right}px`;
      fab.style.bottom = `${bottom}px`;
      dragState.right = right;
      dragState.bottom = bottom;
    });
    fab.addEventListener("pointerup", (event) => {
      if (dragState?.moved && dragState.right != null) {
        // 吸附到指针所在半侧的边缘（18px 标准边距），高度保留用户选择。
        const side = event.clientX < window.innerWidth / 2 ? "left" : "right";
        const maxBottom = Math.max(8, window.innerHeight - 60);
        const bottom = Math.max(8, Math.min(window.innerHeight - event.clientY - dragState.height / 2, maxBottom));
        fabPos = { side, offset: 18, bottom: Math.round(bottom) };
        try { void chrome.storage.local.set({ fabPos }); } catch {}
        applyFabPosition(fab);
      }
      const wasDrag = Boolean(dragState?.moved);
      dragState = null;
      if (wasDrag) suppressNextClick();
    });

    // 拖动结束后浏览器仍会派发 click；用一次性捕获拦截吞掉。
    let clickSuppressed = false;
    const suppressNextClick = () => { clickSuppressed = true; };
    // 单击发送带 260ms 去抖：双击复位时浏览器会派发两次 click，
    // 不去抖会把同一目标发送两次（桌面端 rename 策略会产生重复文件）。
    let clickSendTimer = 0;
    fab.addEventListener("click", (event) => {
      if (clickSuppressed) { clickSuppressed = false; event.stopPropagation(); return; }
      clearTimeout(clickSendTimer);
      clickSendTimer = setTimeout(() => { clickSendTimer = 0; sendToDesktop(); }, 260);
    }, true);

    // 双击：回到默认位置（右下 18px）。
    fab.addEventListener("dblclick", (event) => {
      if (clickSuppressed) return;
      event.preventDefault();
      clearTimeout(clickSendTimer);
      clickSendTimer = 0;
      fabPos = null;
      try { void chrome.storage.local.remove("fabPos"); } catch {}
      fab.style.left = "auto";
      fab.style.right = "18px";
      fab.style.bottom = "18px";
      showBadge("info", "悬浮按钮已回到默认位置");
    });

    // 右键：菜单（复制链接 / 画质 / 隐藏按钮）。
    fab.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (!fabUrl || !isContextValid()) return;
      void openFabMenu();
    });

    const sendToDesktop = () => {
      if (!fabUrl || !isContextValid()) return;
      // page 模式走桌面端 yt-dlp 分析（耗时较长），direct 模式直接下载直链。
      // 直链附带当前页 Referer：多数视频 CDN 校验 Referer，裸请求容易 403。
      label.textContent = fabMode === "page" ? "分析中…" : "发送中…";
      const payload = fabMode === "page"
        ? { type: "download-page-media", url: fabUrl, title: document.title }
        : { type: "send", url: fabUrl, extra: { headers: { Referer: location.href } } };
      void sendMessageSafe(payload).then((response) => {
        const ok = Boolean(response?.ok);
        const errorText = String(response?.error || "发送失败");
        showFabStatus(ok ? "✓ 已发送" : `✕ ${truncate(errorText, 40)}`);
      });
    };
    fab.onkeydown = (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault(); // 阻止默认激活（合成 click），同时阻止空格滚动页面
        if (event.repeat) return; // 按住不放的键盘自动重复不重复发送
        clearTimeout(clickSendTimer);
        clickSendTimer = setTimeout(() => { clickSendTimer = 0; sendToDesktop(); }, 260);
      }
    };
    return fab;
  }

  async function openFabMenu() {
    const items = [{ label: "复制下载链接", value: "copy" }];
    if (fabMode === "page") {
      let mediaQuality = "best";
      try {
        ({ mediaQuality = "best" } = await chrome.storage.local.get("mediaQuality"));
      } catch {}
      for (const [value, text] of [["best", "最高画质"], ["1080", "≤1080p"], ["720", "≤720p"], ["audio", "仅音频"]]) {
        items.push({ label: `画质：${text}`, value: `quality:${value}`, checked: mediaQuality === value });
      }
    }
    items.push({ label: "在此站点隐藏悬浮按钮", value: "hide" });
    try {
      Ui?.showFabMenu?.(fabElement, items, (value) => {
        if (value === "copy") {
          void (navigator.clipboard?.writeText?.(fabUrl) || Promise.reject())
            .then(() => showBadge("info", "已复制链接"))
            .catch(() => showBadge("error", "复制失败：页面不允许访问剪贴板"));
        } else if (value?.startsWith("quality:")) {
          const quality = value.slice("quality:".length);
          try { void chrome.storage.local.set({ mediaQuality: quality }); } catch {}
          showBadge("info", "已切换画质偏好");
        } else if (value === "hide") {
          hideFabForSite();
        }
      });
    } catch {}
  }

  function hideFabForSite() {
    const hostname = location.hostname.toLowerCase();
    if (!hostname) return;
    if (!fabHiddenHosts.includes(hostname)) {
      fabHiddenHosts = [...fabHiddenHosts.slice(-99), hostname];
      try { void chrome.storage.local.set({ fabHiddenHosts }); } catch {}
    }
    showBadge("info", "已在此站点隐藏悬浮按钮，可在扩展弹窗中恢复");
    syncFab();
  }

  function showFabStatus(text) {
    if (!fabLabel) return;
    fabLabel.textContent = text;
    clearTimeout(fabStatusTimer);
    fabStatusTimer = setTimeout(() => { if (fabLabel) fabLabel.textContent = FAB_LABEL; }, 2500);
  }

  // 窗口缩放时重新收拢位置，避免悬浮按钮跑出可视区域。
  window.addEventListener("resize", () => {
    if (fabElement) applyFabPosition(fabElement);
  });

  // ==================== magnet: 链接点击拦截（BT-08） ====================
  // 捕获阶段拦截主键点击与键盘 Enter；把链接交给 background 校验
  // （扩展开关 / 桌面端设置 / 配对 / 在线）。任何前置条件不满足时，
  // 通过 location 赋值把链接交还浏览器原生外部协议处理（§5 离线回退）。
  const isMagnetHref = (href) => /^magnet:/i.test(String(href || "").trim());

  const interceptMagnetActivation = (url) => {
    void sendMessageSafe({ type: "send-magnet", url }).then((response) => {
      if (response?.ok) {
        showBadge("added", "磁力任务已添加到猫步下载器");
        return;
      }
      if (response && response.fallback === false) {
        showBadge("error", `磁力发送失败：${truncate(response.error || "未知错误", 60)}`);
        return;
      }
      // fallback（含 background 不可达）：交还浏览器处理，不丢失用户操作。
      try { window.location.href = url; } catch {}
    });
  };

  document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0) return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const anchor = event.target?.closest?.("a[href]");
    if (!anchor || !isMagnetHref(anchor.getAttribute("href"))) return;
    event.preventDefault();
    event.stopPropagation();
    interceptMagnetActivation(anchor.href);
  }, true);

  document.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const anchor = event.target?.closest?.("a[href]");
    if (!anchor || !isMagnetHref(anchor.getAttribute("href"))) return;
    event.preventDefault();
    event.stopPropagation();
    interceptMagnetActivation(anchor.href);
  }, true);

  // ==================== 消息入口 ====================
  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (message?.type === "show-overlay") {
      // 浮层 UI 与决策状态机在 content-ui.js（P0-5 合并、倒计时、事后记住选择）。
      if (Ui?.handleOverlayMessage) {
        Ui.handleOverlayMessage(message, sendResponse);
      } else {
        try { sendResponse(null); } catch {} // UI 组件缺失时不阻塞接管
      }
      return true; // 异步响应（倒计时/用户点击后才 resolve）
    }
    if (message?.type === "show-badge") {
      const text = message.kind === "added"
        ? `已添加到猫步下载器${message.fileName ? `：${truncate(message.fileName, 48)}` : ""}`
        : String(message.text || "");
      if (text) {
        // 接管成功且带任务 id 时附"撤销"按钮（取消桌面端任务）。
        const options = message.kind === "added" && message.taskId
          ? {
            actions: [{
              label: "撤销", onClick: (btn) => {
                void sendMessageSafe({ type: "task-action", id: message.taskId, action: "cancel" }).then((response) => {
                  const ok = Boolean(response?.ok && response.result?.success);
                  showBadge(ok ? "info" : "error",
                    ok ? "已取消桌面端任务（浏览器下载已结束，可重新发起）"
                      : `撤销失败：${truncate(response?.error || response?.result?.error || "未知错误", 40)}`);
                });
              },
            }],
          }
          : undefined;
        showBadge(message.kind, text, options);
      }
      sendResponse({ ok: true });
      return false;
    }
    if (message?.type === "sniffed-media") {
      sniffedItems = Array.isArray(message.items) ? message.items : [];
      syncFab();
      sendResponse({ ok: true });
      return false;
    }
    return false;
  });

  // ==================== 初始化 ====================
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
      if (document.hidden) return; // 后台标签页暂停周期探测，省 CPU
      collectMedia(true);
    }, 5000);
    // 回到前台立即探测一次，不必等下一个周期（长驻后台的标签页恢复即时性）。
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden && isContextValid()) collectMedia(true);
    });
  }
})();
