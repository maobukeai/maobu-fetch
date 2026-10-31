const MAX_BODY_BYTES = 1024 * 1024;

export function isLanDiskHost(hostname = "") {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  if (host === "localhost" || host === "::1" || /^[a-z0-9-]+$/.test(host)
    || /\.(local|lan|ts\.net|tailscale\.net)$/.test(host)) return true;
  if (/^(fc|fd)[0-9a-f]{2}:|^fe[89ab][0-9a-f]:/.test(host)) return true;
  const bytes = host.split(".").map(Number);
  if (bytes.length !== 4 || bytes.some((n) => !Number.isInteger(n) || n < 0 || n > 255)) return false;
  const [a, b] = bytes;
  return a === 10 || a === 127 || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && b === 168) || (a === 100 && b >= 64 && b <= 127);
}

/** 所有权来自浏览器 sender，不接受网页声称的来源、任意目标或任意认证头。 */
export function validateLanDiskRequest(message, sender, settings, desktopGate) {
  if (!settings.intercept || desktopGate?.enabled === false || Date.now() < Number(settings.bypassUntil || 0)) {
    throw new Error("已暂停接管，请使用网页原生下载");
  }
  if (!Number.isInteger(sender?.tab?.id) || sender.frameId !== 0) throw new Error("只允许当前顶层页面发起打包下载");
  const source = new URL(sender.url || sender.tab.url);
  const target = new URL(message.url);
  const allowed = (settings.allowHosts || []).some((host) => source.hostname === host || source.hostname.endsWith(`.${host}`));
  if (!isLanDiskHost(source.hostname) && !allowed) throw new Error("请先在接管设置中允许此 LanDisk 站点");
  if (!/^https?:$/.test(source.protocol) || target.origin !== source.origin
    || (sender.origin && sender.origin !== source.origin) || target.username || target.password
    || target.pathname !== "/api/download/batch") throw new Error("打包下载只允许当前站点的固定接口");
  if ((settings.blockHosts || []).some((host) => source.hostname === host || source.hostname.endsWith(`.${host}`))
    || Object.entries(settings.siteChoices || {}).some(([host, choice]) => choice === "bypass"
      && (source.hostname === host || source.hostname.endsWith(`.${host}`)))) throw new Error("此站点已选择由浏览器下载");
  if (typeof message.body !== "string" || new TextEncoder().encode(message.body).length > MAX_BODY_BYTES) {
    throw new Error("打包请求为空或过大，请缩小文件选择范围");
  }
  const form = new URLSearchParams(message.body);
  const files = form.getAll("files");
  if (!files.length || files.length > 2000 || form.getAll("pin").length > 1 || form.getAll("folderName").length > 1
    || files.some((file) => !file.trim() || file.length > 8192)
    || [...form.keys()].some((key) => !["files", "folderName", "pin"].includes(key))) throw new Error("打包文件参数无效");
  target.hash = "";
  return {
    url: target.href, body: form.toString(), fileName: String(message.fileName || "打包下载.zip").slice(0, 250),
    pin: form.get("pin") || "", files, fileCount: files.length, origin: source.origin,
  };
}
