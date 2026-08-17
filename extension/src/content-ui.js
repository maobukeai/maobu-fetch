// 猫步下载器内容脚本 UI 组件（经典脚本，manifest 中先于 content.js 注入）。
//
// 承载所有页面内嵌 UI：
//   1. 状态徽章（含"撤销"操作按钮）
//   2. 接管确认浮层：对话式文案、文件大小/来源主机/类型图标、秒数倒计时、
//      深浅色自适应、多下载合并（P0-5）
//   3. 事后"记住选择"轻提示与学习式建议（连续 3 次同决策 → 建议记住）
//   4. FAB 右键菜单（复制链接/画质/隐藏）
//
// 只定义 window.MaobuUi 工厂函数，不注册全局监听，可安全重复注入。
// 使用 inline style 注入，避免触发页面 CSP。
(() => {
  const FONT_STACK = "-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif";
  const truncate = (value, max) => {
    const text = String(value || "");
    return text.length > max ? text.slice(0, max - 1) + "…" : text;
  };

  const prefersDark = () => {
    try { return Boolean(window.matchMedia?.("(prefers-color-scheme: dark)")?.matches); } catch { return false; }
  };

  function formatSize(bytes) {
    const value = Number(bytes || 0);
    if (!value || value < 0) return "";
    if (value < 1024) return `${value} B`;
    if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
    if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
    return `${(value / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  const ICON_GROUPS = [
    ["🗜️", ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"]],
    ["🎬", ["mp4", "mkv", "avi", "webm", "mov", "flv", "ts", "m4v", "wmv"]],
    ["🎵", ["mp3", "flac", "aac", "ogg", "opus", "wav", "m4a", "wma"]],
    ["🖼️", ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "heic"]],
    ["⚙️", ["exe", "msi", "apk", "dmg", "deb", "rpm"]],
    ["📘", ["doc", "docx", "pdf", "txt", "ppt", "pptx", "xls", "xlsx", "epub"]],
  ];
  function fileIconFromName(name) {
    const match = String(name || "").match(/\.([a-z0-9]{1,6})$/i);
    const ext = match ? match[1].toLowerCase() : "";
    for (const [icon, exts] of ICON_GROUPS) {
      if (exts.includes(ext)) return icon;
    }
    return "📄";
  }

  // ==================== 状态徽章 ====================
  let badgeElement = null;
  let badgeTimer = 0;

  /// 轻提示的顶部位置：徽章可见时排在徽章下方，否则用默认 16px。
  /// 两者同用页面右上角，不堆叠会互相遮挡（ask 模式点"用猫步下载"后
  /// 接管成功徽章会盖住"记住选择"轻提示）。
  function followupTopPx() {
    if (badgeElement && badgeElement.isConnected) {
      const rect = badgeElement.getBoundingClientRect();
      return `${Math.round(rect.bottom + 8)}px`;
    }
    return "16px";
  }

  function showBadge(kind, text, options = {}) {
    if (badgeElement) badgeElement.remove();
    clearTimeout(badgeTimer);
    const badge = document.createElement("div");
    badge.id = "maobu-fetch-badge";
    const isError = kind === "error";
    const icon = kind === "added" ? "✓" : isError ? "✕" : "⬇";
    Object.assign(badge.style, {
      position: "fixed", top: "16px", right: "16px", zIndex: "2147483647",
      maxWidth: "320px", padding: "10px 14px", borderRadius: "10px",
      background: isError ? "rgba(64, 20, 22, 0.94)" : "rgba(29, 29, 31, 0.92)",
      color: "#f5f5f7", border: "1px solid rgba(255, 255, 255, 0.16)",
      boxShadow: "0 8px 24px rgba(0, 0, 0, 0.3)",
      fontFamily: FONT_STACK, fontSize: "12px", lineHeight: "1.5",
      display: "flex", flexDirection: "column", gap: "4px",
      transition: "opacity 0.2s, transform 0.2s", opacity: "0", transform: "translateY(-6px)",
    });
    const contentRow = document.createElement("div");
    contentRow.style.display = "flex";
    contentRow.style.gap = "8px";
    contentRow.style.alignItems = "flex-start";
    const iconEl = document.createElement("span");
    iconEl.textContent = icon;
    iconEl.style.color = kind === "added" ? "#6fd18b" : isError ? "#ff8a8a" : "#8fc7ff";
    iconEl.style.fontWeight = "600";
    const textEl = document.createElement("span");
    textEl.textContent = text;
    contentRow.append(iconEl, textEl);
    badge.appendChild(contentRow);

    // 可选操作按钮（如接管成功后的"撤销"），最多 2 个。
    const actions = Array.isArray(options.actions) ? options.actions.slice(0, 2) : [];
    if (actions.length) {
      const row = document.createElement("div");
      row.style.display = "flex";
      row.style.justifyContent = "flex-end";
      row.style.gap = "6px";
      for (const action of actions) {
        const btn = document.createElement("button");
        btn.textContent = action.label || "确定";
        Object.assign(btn.style, {
          padding: "2px 10px", borderRadius: "6px", fontSize: "11px", cursor: "pointer",
          border: "1px solid rgba(255, 255, 255, 0.28)", background: "rgba(255, 255, 255, 0.1)",
          color: "#f5f5f7", fontFamily: FONT_STACK,
        });
        btn.onclick = (event) => {
          event.stopPropagation();
          btn.disabled = true;
          action.onClick?.(btn);
        };
        row.appendChild(btn);
      }
      badge.appendChild(row);
    }

    (document.body || document.documentElement).appendChild(badge);
    badgeElement = badge;
    // 新徽章出现时，把已打开的轻提示挤到徽章下方。
    if (followupElement) followupElement.style.top = followupTopPx();
    requestAnimationFrame(() => {
      badge.style.opacity = "1";
      badge.style.transform = "translateY(0)";
    });
    const duration = isError ? 5000 : actions.length ? 6500 : 3000;
    badgeTimer = setTimeout(() => {
      badge.style.opacity = "0";
      setTimeout(() => {
        badge.remove();
        if (badgeElement === badge) {
          badgeElement = null;
          // 徽章消失后轻提示回到顶部默认位置。
          if (followupElement) followupElement.style.top = "16px";
        }
      }, 220);
    }, duration);
  }

  // ==================== 轻提示（记住选择 / 学习式建议） ====================
  let followupElement = null;
  let followupTimer = 0;

  /// 页面右上角（徽章下方）的小确认条：`onConfirm` 点击"记住"时触发；
  /// `onDismiss` 仅在用户显式点"不了"时触发（超时消失不算拒绝）。
  function showFollowup({ text, confirmLabel = "记住", dismissLabel = "不了", ms = 8000, onConfirm, onDismiss }) {
    if (followupElement) followupElement.remove();
    clearTimeout(followupTimer);
    const chip = document.createElement("div");
    chip.id = "maobu-fetch-followup";
    Object.assign(chip.style, {
      position: "fixed", top: followupTopPx(), right: "16px", zIndex: "2147483646",
      maxWidth: "330px", padding: "10px 14px", borderRadius: "10px",
      background: "rgba(29, 29, 31, 0.92)", color: "#f5f5f7",
      border: "1px solid rgba(255, 255, 255, 0.16)",
      boxShadow: "0 8px 24px rgba(0, 0, 0, 0.28)",
      fontFamily: FONT_STACK, fontSize: "12px", lineHeight: "1.5",
      display: "flex", alignItems: "center", gap: "10px", flexWrap: "wrap",
      transition: "opacity 0.2s, transform 0.2s", opacity: "0", transform: "translateY(-6px)",
    });
    const label = document.createElement("span");
    label.textContent = text;
    label.style.flex = "1 1 auto";
    chip.appendChild(label);
    const makeChipButton = (text, primary) => {
      const btn = document.createElement("button");
      btn.textContent = text;
      Object.assign(btn.style, {
        padding: "2px 10px", borderRadius: "6px", fontSize: "11px", cursor: "pointer",
        fontFamily: FONT_STACK,
        border: primary ? "1px solid rgba(10, 132, 255, 0.6)" : "1px solid rgba(255, 255, 255, 0.28)",
        background: primary ? "rgba(10, 132, 255, 0.25)" : "rgba(255, 255, 255, 0.08)",
        color: "#f5f5f7",
      });
      return btn;
    };
    const confirmBtn = makeChipButton(confirmLabel, true);
    const dismissBtn = makeChipButton(dismissLabel, false);
    const close = () => {
      clearTimeout(followupTimer);
      chip.remove();
      if (followupElement === chip) followupElement = null;
    };
    confirmBtn.onclick = () => { close(); onConfirm?.(); };
    dismissBtn.onclick = () => { close(); onDismiss?.(); };
    chip.append(confirmBtn, dismissBtn);
    (document.body || document.documentElement).appendChild(chip);
    followupElement = chip;
    requestAnimationFrame(() => {
      chip.style.opacity = "1";
      chip.style.transform = "translateY(0)";
    });
    followupTimer = setTimeout(close, ms);
  }

  // ==================== FAB 右键菜单 ====================
  let menuElement = null;
  const onMenuOutside = (event) => {
    if (menuElement && !menuElement.contains(event.target)) closeFabMenu();
  };
  const onMenuKey = (event) => {
    if (event.key === "Escape") closeFabMenu();
  };
  function closeFabMenu() {
    if (menuElement) { menuElement.remove(); menuElement = null; }
    try {
      document.removeEventListener("pointerdown", onMenuOutside, true);
      document.removeEventListener("keydown", onMenuKey, true);
    } catch {}
  }
  /// 在锚点元素上方弹出深色菜单。items：[{label, value, checked?}]；
  /// 选中后回调 onPick(value) 并自动关闭。点击外部 / Esc 关闭。
  function showFabMenu(anchor, items, onPick) {
    closeFabMenu();
    const list = (Array.isArray(items) ? items : []).slice(0, 8);
    if (!list.length) return;
    const menu = document.createElement("div");
    menu.id = "maobu-fetch-fab-menu";
    Object.assign(menu.style, {
      position: "fixed", zIndex: "2147483647", minWidth: "180px",
      padding: "5px", borderRadius: "10px",
      background: "rgba(29, 29, 31, 0.96)", color: "#f5f5f7",
      border: "1px solid rgba(255, 255, 255, 0.16)",
      boxShadow: "0 10px 30px rgba(0, 0, 0, 0.35)",
      fontFamily: FONT_STACK, fontSize: "12px", lineHeight: "1.4",
      display: "flex", flexDirection: "column", gap: "1px",
    });
    let anchorRect = { right: 18, top: 0 };
    try { anchorRect = anchor.getBoundingClientRect(); } catch {}
    menu.style.right = `${Math.max(8, Math.round(window.innerWidth - anchorRect.right))}px`;
    const above = window.innerHeight - anchorRect.top + 8;
    menu.style.bottom = `${Math.max(8, Math.round(above))}px`;
    for (const item of list) {
      const row = document.createElement("div");
      row.setAttribute("role", "menuitem");
      row.setAttribute("tabindex", "0");
      Object.assign(row.style, {
        display: "flex", alignItems: "center", justifyContent: "space-between",
        gap: "12px", padding: "6px 10px", borderRadius: "7px", cursor: "pointer",
      });
      row.onmouseenter = () => { row.style.background = "rgba(255, 255, 255, 0.1)"; };
      row.onmouseleave = () => { row.style.background = "transparent"; };
      const label = document.createElement("span");
      label.textContent = item.label || "";
      row.appendChild(label);
      if (item.checked) {
        const mark = document.createElement("span");
        mark.textContent = "✓";
        mark.style.color = "#6fd18b";
        row.appendChild(mark);
      }
      const pick = () => { closeFabMenu(); onPick?.(item.value); };
      row.onclick = pick;
      row.onkeydown = (event) => {
        if (event.key === "Enter" || event.key === " ") { event.preventDefault(); pick(); }
      };
      menu.appendChild(row);
    }
    (document.body || document.documentElement).appendChild(menu);
    menuElement = menu;
    try {
      document.addEventListener("pointerdown", onMenuOutside, true);
      document.addEventListener("keydown", onMenuKey, true);
    } catch {}
  }

  // ==================== 接管确认浮层 ====================
  // auto 模式：倒计时（默认 1.5 秒，可由设置调整为 0–5000）后自动接管；
  //            "本次用浏览器"可打断。秒数实时可见。
  // ask 模式：等待用户在"用猫步下载器 / 用浏览器下载"间选择，20 秒超时放行浏览器。
  // 同一时刻多个下载到达时合并为一个浮层（显示文件数），批量决策。
  // 显式用户决策后弹出"记住选择"轻提示；连续 3 次同决策时升级为学习式建议。
  const AUTO_TAKEOVER_MS = 1500;
  const ASK_TIMEOUT_MS = 20_000;
  const LEARN_THRESHOLD = 3;
  let overlayState = null;

  function overlayTheme(dark) {
    return dark
      ? {
        background: "rgba(28, 28, 30, 0.92)", border: "1px solid rgba(255, 255, 255, 0.14)",
        text: "#f5f5f7", muted: "#a1a1a6", bar: "rgba(255, 255, 255, 0.12)",
        buttonBorder: "1px solid rgba(255, 255, 255, 0.24)",
        buttonBg: "rgba(255, 255, 255, 0.08)",
        primaryText: "#6fb7ff",
      }
      : {
        background: "rgba(246, 246, 248, 0.9)", border: "1px solid rgba(255, 255, 255, 0.6)",
        text: "#1d1d1f", muted: "#6e6e73", bar: "rgba(0, 0, 0, 0.08)",
        buttonBorder: "1px solid rgba(0, 0, 0, 0.12)",
        buttonBg: "rgba(0, 0, 0, 0.05)",
        primaryText: "#0066cc",
      };
  }

  function createOverlayElement(mode, meta) {
    const dark = prefersDark();
    const theme = overlayTheme(dark);
    const overlay = document.createElement("div");
    overlay.id = "maobu-fetch-takeover-overlay";
    overlay.setAttribute("data-maobu", "1");
    Object.assign(overlay.style, {
      position: "fixed", top: "16px", right: "16px", zIndex: "2147483647",
      maxWidth: "320px", minWidth: "250px", padding: "12px 14px", borderRadius: "10px",
      background: theme.background,
      backdropFilter: "blur(12px) saturate(180%)",
      webkitBackdropFilter: "blur(12px) saturate(180%)",
      border: theme.border,
      boxShadow: "0 10px 30px rgba(0, 0, 0, 0.12), 0 2px 6px rgba(0, 0, 0, 0.05)",
      color: theme.text, fontFamily: FONT_STACK, fontSize: "12px", lineHeight: "1.4",
      display: "flex", flexDirection: "column", gap: "8px",
      transition: "opacity 0.2s, transform 0.2s", opacity: "0", transform: "translateY(-6px)",
    });
    requestAnimationFrame(() => {
      overlay.style.opacity = "1";
      overlay.style.transform = "translateY(0)";
    });

    const title = document.createElement("div");
    title.textContent = mode === "ask" ? "用猫步下载器下载这个文件？" : "交给猫步下载器下载";
    Object.assign(title.style, { fontWeight: "600", color: theme.text, fontSize: "13px", display: "flex", gap: "6px", alignItems: "center" });
    const titleIcon = document.createElement("span");
    titleIcon.textContent = fileIconFromName(meta.fileName);
    title.prepend(titleIcon);
    overlay.appendChild(title);

    const subtitle = document.createElement("div");
    subtitle.className = "maobu-subtitle";
    Object.assign(subtitle.style, {
      color: theme.muted, fontSize: "11px", overflow: "hidden",
      textOverflow: "ellipsis", whiteSpace: "nowrap",
    });
    overlay.appendChild(subtitle);

    // 倒计时进度条 + 实时秒数文本：让"还有多久自动开始/放行"可见。
    const countdown = document.createElement("div");
    countdown.className = "maobu-countdown";
    Object.assign(countdown.style, { color: theme.muted, fontSize: "11px", textAlign: "right" });
    overlay.appendChild(countdown);
    const bar = document.createElement("div");
    bar.className = "maobu-bar";
    Object.assign(bar.style, {
      height: "3px", borderRadius: "2px", background: theme.bar, overflow: "hidden",
    });
    const barFill = document.createElement("span");
    barFill.style.display = "block";
    barFill.style.height = "100%";
    barFill.style.width = "100%";
    barFill.style.background = "#0a84ff";
    bar.appendChild(barFill);
    overlay.appendChild(bar);

    const buttonRow = document.createElement("div");
    buttonRow.style.display = "flex";
    buttonRow.style.justifyContent = "flex-end";
    buttonRow.style.gap = "8px";

    const makeButton = (text, className, primary) => {
      const button = document.createElement("button");
      button.textContent = text;
      button.className = className;
      Object.assign(button.style, {
        padding: "4px 12px", borderRadius: "6px", fontSize: "11px", fontWeight: "500",
        cursor: "pointer", transition: "background-color 0.15s", fontFamily: FONT_STACK,
        border: primary ? "1px solid rgba(10, 132, 255, 0.6)" : theme.buttonBorder,
        background: primary ? "rgba(10, 132, 255, 0.12)" : theme.buttonBg,
        color: primary ? theme.primaryText : theme.text,
      });
      button.onmouseenter = () => { button.style.background = primary ? "rgba(10, 132, 255, 0.22)" : "rgba(127, 127, 127, 0.18)"; };
      button.onmouseleave = () => { button.style.background = primary ? "rgba(10, 132, 255, 0.12)" : theme.buttonBg; };
      return button;
    };

    const bypassBtn = makeButton(mode === "ask" ? "用浏览器下载" : "本次用浏览器", "maobu-bypass", false);
    bypassBtn.onclick = () => resolveOverlay(true, "user");
    buttonRow.appendChild(bypassBtn);

    if (mode === "ask") {
      const takeBtn = makeButton("用猫步下载", "maobu-take", true);
      takeBtn.onclick = () => resolveOverlay(false, "user");
      buttonRow.appendChild(takeBtn);
    }
    overlay.appendChild(buttonRow);

    // Esc = 本次由浏览器下载（键盘用户直觉操作）。
    overlay.tabIndex = -1;
    overlay.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        resolveOverlay(true, "user");
      }
    });
    return overlay;
  }

  function updateOverlaySubtitle() {
    if (!overlayState) return;
    const subtitle = overlayState.element.querySelector(".maobu-subtitle");
    if (!subtitle) return;
    const meta = overlayState;
    if (meta.count > 1) {
      subtitle.textContent = `共 ${meta.count} 个文件将被接管`;
      return;
    }
    const parts = [truncate(meta.fileNames[0], 60) || "本次下载将转交桌面端处理"];
    const details = [meta.host, formatSize(meta.sizeBytes)].filter(Boolean);
    if (details.length) parts.push(details.join(" · "));
    subtitle.textContent = parts.join(" · ");
  }

  function startOverlayCountdown() {
    if (!overlayState) return;
    const state = overlayState;
    const duration = state.mode === "ask" ? ASK_TIMEOUT_MS
      : Math.max(0, Number(state.delayMs ?? AUTO_TAKEOVER_MS) || AUTO_TAKEOVER_MS);
    const deadline = Date.now() + duration;
    const label = state.element.querySelector(".maobu-countdown");
    const barFill = state.element.querySelector(".maobu-bar > span");
    const tick = () => {
      if (overlayState !== state) return;
      const remaining = Math.max(0, deadline - Date.now());
      const seconds = Math.ceil(remaining / 1000);
      if (label) {
        label.textContent = state.mode === "ask"
          ? `${seconds} 秒后由浏览器下载`
          : remaining === 0 ? "" : `${seconds} 秒后自动开始`;
      }
      if (remaining <= 0) return;
    };
    tick();
    state.tickId = setInterval(tick, 250);
    if (barFill && duration > 0) {
      barFill.style.transition = "none";
      barFill.style.width = "100%";
      void barFill.offsetWidth; // 强制 reflow，重启动画
      barFill.style.transition = `width ${duration}ms linear`;
      barFill.style.width = "0%";
    }
    state.timeoutId = setTimeout(() => {
      // auto：超时自动接管；ask：超时放行浏览器（安全回退，不丢下载）。
      resolveOverlay(state.mode === "ask", "timeout");
    }, duration);
  }

  function stopOverlayTimers(state) {
    clearInterval(state.tickId);
    clearTimeout(state.timeoutId);
  }

  function restartOverlayCountdown() {
    if (!overlayState) return;
    stopOverlayTimers(overlayState);
    // 合并新下载时重置倒计时（auto 模式），给用户完整窗口期做决策。
    if (overlayState.mode === "ask") return;
    startOverlayCountdown();
  }

  function resolveOverlay(bypass, source = "timeout") {
    if (!overlayState) return;
    const state = overlayState;
    stopOverlayTimers(state);
    overlayState = null;
    for (const waiter of state.waiters) {
      try { waiter({ bypass }); } catch {}
    }
    state.element.remove();
    // 只有用户显式点击才询问"记住选择"；倒计时自动决策不打扰。
    if (source === "user") void maybeFollowupAfterDecision(state.host, bypass);
  }

  function joinOverlay(message, sendResponse) {
    if (overlayState && overlayState.element.isConnected) {
      overlayState.waiters.push(sendResponse);
      overlayState.count += 1;
      overlayState.fileNames.push(message.fileName || "");
      updateOverlaySubtitle();
      restartOverlayCountdown();
      return;
    }
    const mode = message.mode === "ask" ? "ask" : "auto";
    overlayState = {
      mode, count: 1,
      fileNames: [message.fileName || ""],
      host: String(message.host || location.hostname || ""),
      sizeBytes: Number(message.sizeBytes || 0),
      delayMs: Number(message.delayMs ?? AUTO_TAKEOVER_MS),
      waiters: [sendResponse],
      element: createOverlayElement(mode, { fileName: message.fileName || "" }),
      timeoutId: 0, tickId: 0,
    };
    (document.body || document.documentElement).appendChild(overlayState.element);
    updateOverlaySubtitle();
    startOverlayCountdown();
    // 默认焦点：auto → 绕过按钮（一键打断）；ask → 主按钮（回车即接管）。
    const focusTarget = overlayState.element.querySelector(mode === "ask" ? ".maobu-take" : ".maobu-bypass");
    focusTarget?.focus({ preventScroll: true });
  }

  // ---- 学习式站点记忆 ----
  // 每次显式决策计数；第 1 次问"记住吗"，达到阈值（3 次）时改问
  // "最近 n 次都…，要记住吗"；用户点"不了"后该站点不再打扰。
  async function maybeFollowupAfterDecision(host, bypass) {
    if (!host) return;
    let counts = {};
    let dismissed = {};
    try {
      const stored = await chrome.storage.local.get(["siteDecisionCounts", "sitePromptDismissed"]);
      counts = stored.siteDecisionCounts || {};
      dismissed = stored.sitePromptDismissed || {};
    } catch { return; }
    const kind = bypass ? "bypass" : "take";
    const hostCounts = counts[host] || { take: 0, bypass: 0 };
    hostCounts[kind] += 1;
    hostCounts[kind === "bypass" ? "take" : "bypass"] = 0; // 反向决策重置计数
    counts[host] = hostCounts;
    try { await chrome.storage.local.set({ siteDecisionCounts: counts }); } catch {}

    if (dismissed[host]) return;
    const n = hostCounts[kind];
    const decisionText = bypass ? "用浏览器下载" : "用猫步下载器下载";
    const isSuggestion = n >= LEARN_THRESHOLD;
    showFollowup({
      text: isSuggestion
        ? `最近 ${n} 次都${decisionText} ${host} 的文件，要记住这个选择吗？`
        : `记住对 ${host} 的选择（${decisionText}）吗？`,
      onConfirm: async () => {
        try {
          const { siteChoices = {} } = await chrome.storage.local.get("siteChoices");
          siteChoices[host] = kind;
          delete counts[host];
          await chrome.storage.local.set({ siteChoices, siteDecisionCounts: counts });
          showBadge("info", `已记住：${host} ${decisionText}`);
        } catch {}
      },
      onDismiss: isSuggestion
        ? async () => {
          dismissed[host] = true;
          try { await chrome.storage.local.set({ sitePromptDismissed: dismissed }); } catch {}
        }
        : null,
    });
  }

  window.MaobuUi = {
    formatSize,
    fileIconFromName,
    prefersDark,
    showBadge,
    showFollowup,
    showFabMenu,
    closeFabMenu,
    handleOverlayMessage: joinOverlay,
  };
})();
