const id = decodeURIComponent(location.hash.slice(1));
const status = document.getElementById("status");
const accept = document.getElementById("accept");
const decline = document.getElementById("decline");
async function choose(accepted, event) {
  if (!event.isTrusted) return;
  accept.disabled = decline.disabled = true;
  try {
    const result = await chrome.runtime.sendMessage({ type: "landisk-approval-choice", id, accepted });
    if (!result?.ok) { status.textContent = result?.error || "确认失败，请返回网页重新下载"; }
  } catch { status.textContent = "扩展连接已断开，请使用网页下载"; }
}
accept.addEventListener("click", (event) => void choose(true, event));
decline.addEventListener("click", (event) => void choose(false, event));
try {
  const details = await chrome.runtime.sendMessage({ type: "landisk-approval-details", id });
  if (!details?.ok) throw new Error(details?.error || "请求已失效");
  document.getElementById("summary").textContent = `${details.result.fileName}（${details.result.fileCount} 个文件）`;
  document.getElementById("origin").textContent = details.result.origin;
  for (const file of details.result.files) {
    const item = document.createElement("li");
    item.textContent = file;
    document.getElementById("files").append(item);
  }
  accept.disabled = false;
  decline.focus();
} catch (error) { status.textContent = error.message; }
