// 划词多链接预览页：右键"下载选中文字中的链接"识别出多条链接时，
// 由 background 打开本页，先勾选确认再逐条发送（避免一次性批量添加任务）。
// 链接数据经 location.hash 传入（encodeURIComponent(JSON)），无网络请求。
import { isDownloadableMagnet } from "./links.js";

/// 解析 hash 载荷：`#<encodeURIComponent(JSON.stringify(string[]))>`。
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
    const link = String(item || "").trim();
    if (!link || seen.has(link)) continue;
    if (!isDownloadableMagnet(link) && !/^https?:\/\//i.test(link)) continue;
    seen.add(link);
    links.push(link);
  }
  return links;
}

function labelFor(link) {
  if (isDownloadableMagnet(link)) return { icon: "🧲", text: "磁力链接" };
  let name = "";
  try {
    const url = new URL(link);
    try {
      name = decodeURIComponent(url.pathname.split("/").pop() || "") || url.hostname;
    } catch { name = url.pathname.split("/").pop() || url.hostname; } // 非法 % 序列时保留原始段
  } catch { name = link; }
  return { icon: "🔗", text: name };
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
  const countEl = document.getElementById("linkCount");
  if (!listEl || !sendBtn) return;

  const links = parseSelectionHash(location.hash);
  const rows = [];
  // Esc 关闭（与取消按钮等效；键盘用户不必移到按钮上）。
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") window.close();
  });
  if (!links.length) {
    statusEl.textContent = "没有可发送的链接，本窗口可以关闭。";
    sendBtn.disabled = true;
    document.getElementById("cancel").onclick = () => window.close();
    return;
  }
  countEl.textContent = `${links.length} 条链接`;

  for (const link of links) {
    const row = document.createElement("label");
    row.className = "link-item";
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = true;
    const { icon, text } = labelFor(link);
    const nameEl = document.createElement("b");
    nameEl.textContent = text;
    nameEl.title = link;
    const urlEl = document.createElement("small");
    urlEl.textContent = truncateMiddle(link, 56);
    urlEl.title = link;
    const info = document.createElement("div");
    info.className = "link-info";
    info.append(nameEl, urlEl);
    row.append(checkbox, document.createTextNode(icon), info);
    listEl.appendChild(row);
    rows.push({ checkbox, link, row });
  }

  const updateSendState = () => {
    const checked = rows.filter((item) => item.checkbox.checked).length;
    sendBtn.disabled = checked === 0;
    sendBtn.textContent = `下载所选（${checked}）`;
    if (selectAllEl) {
      selectAllEl.checked = checked === rows.length;
    }
  };
  for (const { checkbox } of rows) checkbox.addEventListener("change", updateSendState);
  if (selectAllEl) {
    selectAllEl.addEventListener("change", () => {
      for (const { checkbox } of rows) checkbox.checked = selectAllEl.checked;
      updateSendState();
    });
  }
  updateSendState();

  document.getElementById("cancel").onclick = () => window.close();
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
