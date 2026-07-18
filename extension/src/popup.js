const $ = id => document.getElementById(id);
const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
const stored = await chrome.storage.local.get(["intercept", "minSizeMb"]);
$("intercept").checked = stored.intercept ?? true;
$("minSize").value = String(stored.minSizeMb ?? 1);

chrome.runtime.sendMessage({ type: "health" }, response => {
  const online = !!response?.ok;
  $("status").classList.toggle("online", online);
  $("message").textContent = online ? "LumaGet 已连接，可以开始下载" : "桌面端未连接，浏览器下载不会被中断";
});

const key = `media:${tab?.id}`;
const session = await chrome.storage.session.get(key);
const items = session[key] || [];
$("count").textContent = items.length;
if (items.length) {
  $("media").innerHTML = "";
  items.slice(0, 8).forEach(item => {
    const row = document.createElement("div"); row.className = "media-item";
    const name = item.url.split("/").pop()?.split("?")[0] || item.title || "媒体资源";
    row.innerHTML = `<i>↓</i><div><b></b><span>${item.type}</span></div><button title="发送到 LumaGet">+</button>`;
    row.querySelector("b").textContent = name;
    row.querySelector("button").onclick = () => send(item.url, name);
    $("media").append(row);
  });
}

async function send(url, fileName) {
  if (!url) return;
  chrome.runtime.sendMessage({ type: "send", url, fileName }, response => {
    $("message").textContent = response?.ok ? "已发送，桌面端开始下载" : `发送失败：${response?.error || "请打开桌面端"}`;
  });
}
$("send").onclick = () => send($("url").value.trim());
$("url").onkeydown = event => { if (event.key === "Enter") $("send").click(); };
$("intercept").onchange = event => chrome.storage.local.set({ intercept: event.target.checked });
$("minSize").onchange = event => chrome.storage.local.set({ minSizeMb: Number(event.target.value) });

