import { signedFetch } from "./protocol.js";

const PAN_HOSTS = new Set(["pan.baidu.com", "yun.baidu.com"]);
const CDN_HOSTS = ["pcs.baidu.com", "baidupcs.com"];
const DOWNLOAD_HEADERS = new Set(["cookie", "user-agent", "referer", "accept", "accept-language"]);

function parseUrl(value) {
  try { return new URL(value); } catch { return null; }
}

function isDefaultHttps(url) {
  return url?.protocol === "https:" && !url.port && !url.username && !url.password;
}

function isPanUrl(value) {
  const url = parseUrl(value);
  return isDefaultHttps(url) && PAN_HOSTS.has(url.hostname);
}

function isBaiduDownloadHost(host) {
  return PAN_HOSTS.has(host) || CDN_HOSTS.some((domain) => host === domain || host.endsWith(`.${domain}`));
}

export function isBaiduDownload(item, tab) {
  // findSourceTab may return an unrelated active tab for the confirmation overlay.
  const tabUrl = item.tabId != null && item.tabId === tab?.id ? tab.url : undefined;
  return [item.url, item.finalUrl].some((value) => isBaiduDownloadHost(parseUrl(value)?.hostname || ""))
    || [item.referrer, item.origin, tabUrl].some(isPanUrl);
}

function isUnsafeResponse(item) {
  if (item.method && item.method.toUpperCase() !== "GET") return true;
  const mime = (item.mime || "").split(";")[0].trim().toLowerCase();
  return mime === "text/html" || mime === "application/xhtml+xml" || mime === "application/json"
    || mime === "text/json" || mime.endsWith("+json") || mime.startsWith("multipart/");
}

export function baiduTakeoverBlockReason(item, initial) {
  const target = parseUrl(item.finalUrl);
  if (!isDefaultHttps(target) || !isBaiduDownloadHost(target.hostname)) return "baidu-unverified-target";
  if (!Number.isSafeInteger(item.totalBytes) || item.totalBytes <= 0) return "baidu-size-unknown";
  if (isUnsafeResponse(item) || isUnsafeResponse(initial)) return "baidu-browser-only";
  return null;
}

export function baiduDownloadHeaders(headers, initial) {
  const allowed = Object.fromEntries(Object.entries(headers).filter(([name, value]) => {
    const key = name.toLowerCase();
    return DOWNLOAD_HEADERS.has(key) && (key !== "referer" || isPanUrl(value));
  }));
  if (!Object.keys(allowed).some((name) => name.toLowerCase() === "referer") && isPanUrl(initial.referrer)) {
    allowed.Referer = initial.referrer;
  }
  return allowed;
}

export async function verifyBaiduDownload(url, headers, totalBytes) {
  const response = await signedFetch("/v1/tasks/verify-baidu-download", { url, headers, total_bytes: totalBytes }, { timeoutMs: 5_000 });
  return response.status === 204;
}
