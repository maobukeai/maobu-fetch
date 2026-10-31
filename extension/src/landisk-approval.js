/** 确认页属于扩展 origin，宿主页无法遮盖控件或伪造其消息身份。 */
export function createLanDiskApproval({ chrome, timeoutMs = 45_000, randomId = () => crypto.randomUUID() }) {
  const pending = new Map();
  const approvalUrl = chrome.runtime.getURL?.("src/landisk-approval.html");
  const finish = (id, accepted) => {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    clearTimeout(entry.timer);
    entry.resolve(accepted);
    if (entry.windowId !== undefined) void Promise.resolve(chrome.windows?.remove(entry.windowId)).catch(() => {});
  };
  chrome.windows?.onRemoved?.addListener?.((windowId) => {
    for (const [id, entry] of pending) if (entry.windowId === windowId) finish(id, false);
  });
  const cancelTab = (tabId) => {
    for (const [id, entry] of pending) if (entry.sender.tab.id === tabId) finish(id, false);
  };
  chrome.tabs?.onRemoved?.addListener?.(cancelTab);
  chrome.tabs?.onUpdated?.addListener?.((id, change) => {
    if (change.url || change.status === "loading") cancelTab(id);
  });
  return {
    async ask(request, sender) {
      if (!approvalUrl || !chrome.windows?.create) throw new Error("无法打开安全确认窗口，请使用网页下载");
      if (pending.size >= 16 || [...pending.values()].some((entry) => entry.sender.tab.id === sender.tab.id)) {
        throw new Error("请先处理当前打包确认");
      }
      const id = randomId();
      const entry = { request, sender };
      entry.result = new Promise((resolve) => { entry.resolve = resolve; });
      entry.timer = setTimeout(() => finish(id, false), timeoutMs);
      pending.set(id, entry);
      entry.opening = (async () => {
        try {
          const popup = await chrome.windows.create({ url: `${approvalUrl}#${encodeURIComponent(id)}`, type: "popup", width: 440, height: 370, focused: true, incognito: Boolean(sender.tab.incognito) });
          entry.windowId = popup.id;
          entry.tabId = popup.tabs?.[0]?.id ?? (await chrome.tabs.query({ windowId: popup.id }))?.[0]?.id;
          if (!pending.has(id)) void Promise.resolve(chrome.windows.remove(popup.id)).catch(() => {});
        } catch { finish(id, false); }
      })();
      await entry.opening;
      return entry.result;
    },
    async message(message, sender) {
      const entry = pending.get(message.id);
      if (!entry) return { ok: false, error: "确认已过期，请重新发起下载" };
      await entry.opening;
      if (sender.url?.split("#")[0] !== approvalUrl || sender.tab?.id !== entry.tabId) {
        return { ok: false, error: "只有扩展确认窗口可以批准此下载" };
      }
      if (message.type === "landisk-approval-details") return { ok: true, result: {
        fileName: entry.request.fileName, fileCount: entry.request.fileCount, origin: entry.request.origin,
        files: entry.request.files.slice(0, 5),
      } };
      if (message.type !== "landisk-approval-choice") return { ok: false, error: "无效确认操作" };
      finish(message.id, message.accepted === true);
      return { ok: true };
    },
  };
}
