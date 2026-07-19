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
  const statusEl = $("status");
  if (statusEl) statusEl.classList.toggle("online", online);
  const connEl = $("connection");
  if (connEl) connEl.textContent = online ? "桌面端已连接" : "桌面端未连接";
  const pairBoxEl = $("pairBox");
  if (pairBoxEl) pairBoxEl.classList.toggle("hidden", !online || response?.paired);
  message(!online ? "请先启动猫步下载器；下载会保留在浏览器中" : response?.paired ? "连接安全，可以发送下载" : "需要先输入桌面端配对码", !online);
}
await health().catch(() => {});
const refreshEl = $("refresh");
if (refreshEl) refreshEl.onclick = health;

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
      chrome.runtime.openOptionsPage(() => {
        if (chrome.runtime.lastError) {
          chrome.tabs.create({ url: "options.html" });
        }
      });
    } catch {
      try {
        chrome.tabs.create({ url: "options.html" });
      } catch {}
    }
  };
}

const bypassEl = $("bypass");
if (bypassEl) {
  bypassEl.onclick = async () => {
    await call({ type: "bypass", minutes: 10 });
    message("接管已暂停 10 分钟");
  };
}

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
  };
}


