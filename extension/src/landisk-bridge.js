// 网页消息只能提出请求；用户在扩展自己的确认窗口中批准后，后台才下发下载。
(() => {
  if (!globalThis.chrome?.runtime?.id || window.__maobuLanDiskBridgeListening) return;
  window.__maobuLanDiskBridgeListening = true;
  let pending = false;
  const send = (message) => new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage(message, (result) => resolve(chrome.runtime.lastError ? null : result));
    } catch { resolve(null); }
  });

  window.addEventListener("message", async (event) => {
    const data = event.data;
    if (event.source !== window || event.origin !== location.origin || data?.source !== "maobu-landisk-main"
      || data.type !== "SEND_LANDISK_BATCH_TASK" || typeof data.reqId !== "string") return;
    const reply = (ok, error = "") => window.postMessage({
      source: "maobu-landisk-isolated", type: "LANDISK_BATCH_RESULT", reqId: data.reqId, ok, error,
    }, location.origin);
    if (pending) return reply(false, "请先处理当前下载确认");
    pending = true;
    try {
      const payload = data.payload || {};
      const result = await send({ ...payload, type: "send-landisk-batch-task" });
      reply(Boolean(result?.ok), result?.error || "");
    } finally { pending = false; }
  });

  function addDownloadButton() {
    const original = document.getElementById("btn-batch-zip");
    if (!original || document.getElementById("btn-maobu-batch-turbo")) return;
    const button = document.createElement("button");
    button.id = "btn-maobu-batch-turbo";
    button.type = "button";
    button.textContent = "猫步打包下载";
    button.style.cssText = "margin-left:6px;padding:5px 10px;border:1px solid #176bc6;border-radius:4px;background:#176bc6;color:#fff;cursor:pointer";
    button.addEventListener("click", (event) => { if (event.isTrusted) original.click(); });
    original.insertAdjacentElement("afterend", button);
  }
  let queued = false;
  const observer = new MutationObserver(() => {
    if (queued) return;
    queued = true;
    queueMicrotask(() => { queued = false; addDownloadButton(); });
  });
  const observe = () => { if (document.documentElement) observer.observe(document.documentElement, { childList: true, subtree: true }); };
  if (document.documentElement) observe();
  else document.addEventListener("DOMContentLoaded", observe, { once: true });
  addDownloadButton();
})();
