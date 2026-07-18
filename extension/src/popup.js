const $ = (id) => document.getElementById(id);
const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
const stored = await chrome.storage.local.get(["intercept", "minSizeMb", "bypassUntil"]);
$("intercept").checked = stored.intercept ?? true; $("minSize").value = String(stored.minSizeMb ?? 1);

function message(text, error = false) { $("message").textContent = text; $("message").classList.toggle("error", error); }
function call(payload) { return new Promise((resolve) => chrome.runtime.sendMessage(payload, resolve)); }
async function health() {
  const response = await call({ type: "health" }); const online = Boolean(response?.ok);
  $("status").classList.toggle("online", online); $("connection").textContent = online ? "桌面端已连接" : "桌面端未连接";
  $("pairBox").classList.toggle("hidden", !online || response?.paired); message(!online ? "请先启动 LumaGet；下载会保留在浏览器中" : response?.paired ? "连接安全，可以发送下载" : "需要先输入桌面端配对码", !online);
}
await health();
$("refresh").onclick = health;
$("pair").onclick = async () => { const code = $("pairCode").value.trim(); if (!/^\d{6}$/.test(code)) return message("请输入 6 位配对码", true); const result = await call({ type: "pair", code }); if (result?.ok) { message("配对成功"); await health(); } else message(`配对失败：${result?.error || "未知错误"}`, true); };

const key = `media:${tab?.id}`; const session = await chrome.storage.session.get(key); const items = session[key] || [];
$("count").textContent = items.length;
if (items.length) { $("media").innerHTML = ""; items.slice(-10).reverse().forEach((item) => { const row = document.createElement("div"); row.className = "media-item"; const name = decodeURIComponent(item.url.split("/").pop()?.split("?")[0] || item.title || "媒体资源").slice(0, 80); row.innerHTML = "<i>↓</i><div><b></b><small></small></div><button title='发送'>＋</button>"; row.querySelector("b").textContent = name; row.querySelector("small").textContent = item.type; row.querySelector("button").onclick = () => send(item.url, name); $("media").append(row); }); }
async function send(url, fileName) { if (!url) return; message("正在发送…"); const response = await call({ type: "send", url, fileName }); message(response?.ok ? "已发送到桌面端" : `发送失败：${response?.error || "请检查配对状态"}`, !response?.ok); }
$("send").onclick = () => send($("url").value.trim()); $("url").onkeydown = (event) => { if (event.key === "Enter") $("send").click(); };
$("intercept").onchange = (event) => chrome.storage.local.set({ intercept: event.target.checked });
$("minSize").onchange = (event) => chrome.storage.local.set({ minSizeMb: Number(event.target.value) });
$("bypass").onclick = async () => { await call({ type: "bypass", minutes: 10 }); message("接管已暂停 10 分钟"); };
