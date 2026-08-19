// 猫步资源与批量链接提取器（由 background 以独立弹窗打开）。
// 支持：
//   1. 划词选中文本链接预览与发送
//   2. 网页全量资源提取（音视频、压缩包、镜像、图片、文档、磁力）
//   3. 按分类芯片（全部/视频/音频/压缩包/软件/图片/文档/磁力）一键过滤
//   4. 实时关键词 / 正则表达式筛选
//   5. 全选 / 反选 / 快捷勾选与批量发送到猫步桌面端
import { isDownloadableMagnet } from "./links.js";

const CATEGORY_MAP = {
  video: { label: "视频", icon: "🎬", exts: ["mp4", "mkv", "avi", "webm", "mov", "flv", "ts", "m4v", "wmv", "m3u8", "mpd"] },
  audio: { label: "音频", icon: "🎵", exts: ["mp3", "flac", "aac", "ogg", "opus", "wav", "m4a", "wma"] },
  archive: { label: "压缩包", icon: "🗜️", exts: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso", "tgz"] },
  installer: { label: "安装包", icon: "⚙️", exts: ["exe", "msi", "apk", "dmg", "deb", "rpm", "pkg"] },
  image: { label: "图片", icon: "🖼️", exts: ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico"] },
  doc: { label: "文档", icon: "📘", exts: ["doc", "docx", "pdf", "txt", "ppt", "pptx", "xls", "xlsx", "epub", "mobi", "md"] },
};

export function categorizeLink(link) {
  if (isDownloadableMagnet(link)) return { category: "magnet", icon: "🧲", label: "磁力" };
  let ext = "";
  try {
    const parsed = new URL(link);
    const pathname = parsed.pathname || "";
    const match = pathname.match(/\.([a-z0-9]{1,6})$/i);
    if (match) ext = match[1].toLowerCase();
  } catch {
    const match = String(link).match(/\.([a-z0-9]{1,6})($|\?)/i);
    if (match) ext = match[1].toLowerCase();
  }
  for (const [key, conf] of Object.entries(CATEGORY_MAP)) {
    if (conf.exts.includes(ext)) {
      return { category: key, icon: conf.icon, label: conf.label };
    }
  }
  return { category: "other", icon: "🔗", label: "链接" };
}

/// 解析 hash 载荷：`#<encodeURIComponent(JSON.stringify(string[] | object[]))>`。
/// 返回去重后的 http(s)/magnet 链接数组；载荷无效时返回空数组。
export function parseSelectionHash(hash) {
  const raw = String(hash || "").replace(/^#/, "");
  if (!raw) return [];
  let parsed;
  try { parsed = JSON.parse(decodeURIComponent(raw)); } catch { return []; }
  if (!Array.isArray(parsed)) return [];
  const seen = new Set();
  const links = [];
  for (const item of parsed) {
    const link = typeof item === "string" ? item.trim() : String(item?.url || "").trim();
    if (!link || seen.has(link)) continue;
    if (!isDownloadableMagnet(link) && !/^https?:\/\//i.test(link)) continue;
    seen.add(link);
    links.push(link);
  }
  return links;
}

function labelFor(link) {
  const cat = categorizeLink(link);
  if (isDownloadableMagnet(link)) return { icon: "🧲", text: "磁力链接", category: "magnet" };
  let name = "";
  try {
    const url = new URL(link);
    try {
      name = decodeURIComponent(url.pathname.split("/").pop() || "") || url.hostname;
    } catch { name = url.pathname.split("/").pop() || url.hostname; }
  } catch { name = link; }
  return { icon: cat.icon, text: name || link, category: cat.category };
}

function truncateMiddle(text, max) {
  if (text.length <= max) return text;
  const half = Math.floor((max - 1) / 2);
  return `${text.slice(0, half)}…${text.slice(text.length - half)}`;
}

const call = (payload) => new Promise((resolve) => {
  try {
    chrome.runtime.sendMessage(payload, resolve);
  } catch {
    resolve(null);
  }
});

function init() {
  const listEl = document.getElementById("selectionList");
  const statusEl = document.getElementById("status");
  const sendBtn = document.getElementById("send");
  const selectAllEl = document.getElementById("selectAll");
  const invertBtn = document.getElementById("invertSelection");
  const countEl = document.getElementById("linkCount");
  const searchInput = document.getElementById("searchFilter");
  const chipsEl = document.getElementById("categoryChips");
  const filteredHint = document.getElementById("filteredHint");
  if (!listEl || !sendBtn) return;

  const links = parseSelectionHash(location.hash);
  // Esc 关闭
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") window.close();
  });
  if (!links.length) {
    statusEl.textContent = "没有可发送的资源或链接，本窗口可以关闭。";
    sendBtn.disabled = true;
    const cancelBtn = document.getElementById("cancel");
    if (cancelBtn) cancelBtn.onclick = () => window.close();
    return;
  }
  countEl.textContent = `${links.length} 条资源`;

  let activeCategory = "all";
  let searchKeyword = "";

  const rows = links.map((link) => {
    const { icon, text, category } = labelFor(link);
    const row = document.createElement("label");
    row.className = "link-item";

    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = true;

    const badge = document.createElement("span");
    badge.className = "link-badge";
    badge.textContent = icon;

    const nameEl = document.createElement("b");
    nameEl.textContent = text;
    nameEl.title = link;

    const urlEl = document.createElement("small");
    urlEl.textContent = truncateMiddle(link, 60);
    urlEl.title = link;

    const info = document.createElement("div");
    info.className = "link-info";
    info.append(nameEl, urlEl);

    row.append(checkbox, badge, info);
    listEl.appendChild(row);

    return { checkbox, link, text, category, row, visible: true };
  });

  // 渲染分类 Chips
  const categoryCounts = { all: rows.length };
  for (const item of rows) {
    categoryCounts[item.category] = (categoryCounts[item.category] || 0) + 1;
  }

  const chipDefs = [
    { key: "all", label: "全部", icon: "🌐" },
    { key: "video", label: "视频", icon: "🎬" },
    { key: "audio", label: "音频", icon: "🎵" },
    { key: "archive", label: "压缩包", icon: "🗜️" },
    { key: "installer", label: "安装包", icon: "⚙️" },
    { key: "image", label: "图片", icon: "🖼️" },
    { key: "doc", label: "文档", icon: "📘" },
    { key: "magnet", label: "磁力", icon: "🧲" },
  ];

  if (chipsEl) {
    for (const def of chipDefs) {
      const count = categoryCounts[def.key] || 0;
      if (def.key !== "all" && count === 0) continue;
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = `chip ${def.key === activeCategory ? "active" : ""}`;
      chip.innerHTML = `${def.icon} ${def.label} <span class="chip-count">${count}</span>`;
      chip.onclick = () => {
        activeCategory = def.key;
        chipsEl.querySelectorAll(".chip").forEach((el) => el.classList.remove("active"));
        chip.classList.add("active");
        applyFilters();
      };
      chipsEl.appendChild(chip);
    }
  }

  function applyFilters() {
    let regex = null;
    if (searchKeyword) {
      try {
        regex = new RegExp(searchKeyword, "i");
      } catch {
        regex = null;
      }
    }

    let visibleCount = 0;
    for (const item of rows) {
      const catMatch = activeCategory === "all" || item.category === activeCategory;
      let textMatch = true;
      if (searchKeyword) {
        if (regex) {
          textMatch = regex.test(item.text) || regex.test(item.link);
        } else {
          const lower = searchKeyword.toLowerCase();
          textMatch = item.text.toLowerCase().includes(lower) || item.link.toLowerCase().includes(lower);
        }
      }
      item.visible = catMatch && textMatch;
      item.row.style.display = item.visible ? "flex" : "none";
      if (item.visible) visibleCount += 1;
    }

    if (filteredHint) {
      filteredHint.textContent = `显示 ${visibleCount} / ${rows.length} 条`;
    }
    updateSendState();
  }

  if (searchInput) {
    searchInput.addEventListener("input", () => {
      searchKeyword = searchInput.value.trim();
      applyFilters();
    });
  }

  const updateSendState = () => {
    const visibleRows = rows.filter((item) => item.visible);
    const checked = visibleRows.filter((item) => item.checkbox.checked).length;
    const totalChecked = rows.filter((item) => item.checkbox.checked).length;
    sendBtn.disabled = totalChecked === 0;
    sendBtn.textContent = `下载所选（${totalChecked}）`;
    if (selectAllEl) {
      selectAllEl.checked = visibleRows.length > 0 && checked === visibleRows.length;
    }
  };

  for (const { checkbox } of rows) {
    checkbox.addEventListener("change", updateSendState);
  }

  if (selectAllEl) {
    selectAllEl.addEventListener("change", () => {
      for (const item of rows) {
        if (item.visible) item.checkbox.checked = selectAllEl.checked;
      }
      updateSendState();
    });
  }

  if (invertBtn) {
    invertBtn.addEventListener("click", () => {
      for (const item of rows) {
        if (item.visible) item.checkbox.checked = !item.checkbox.checked;
      }
      updateSendState();
    });
  }

  updateSendState();

  const cancelBtn = document.getElementById("cancel");
  if (cancelBtn) cancelBtn.onclick = () => window.close();

  sendBtn.onclick = async () => {
    const targets = rows.filter((item) => item.checkbox.checked);
    sendBtn.disabled = true;
    let added = 0;
    let failed = 0;
    for (const item of targets) {
      item.row.classList.add("pending");
      const response = await call({ type: "send", url: item.link });
      if (response?.ok) {
        added += 1;
        item.row.classList.add("ok");
      } else {
        failed += 1;
        item.row.classList.add("fail");
        item.row.title = String(response?.error || "发送失败");
      }
      item.row.classList.remove("pending");
    }
    statusEl.textContent = failed
      ? `已添加 ${added} 个任务，${failed} 个失败（悬停查看原因）`
      : `已添加 ${added} 个任务，窗口即将关闭…`;
    if (!failed) setTimeout(() => window.close(), 1200);
    else sendBtn.disabled = false;
  };
}

// 仅在真实 DOM 环境（扩展页面）初始化；Node 测试环境只导入纯函数。
if (typeof document !== "undefined" && document.getElementById("selectionList")) {
  init();
}
