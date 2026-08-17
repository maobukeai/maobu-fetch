export const API = "http://127.0.0.1:17433";
const bytesToHex = (bytes) => [...bytes].map((value) => value.toString(16).padStart(2, "0")).join("");

// 每请求超时（毫秒）。本地桥正常响应在毫秒级；超时说明桌面端进程僵死
// （TCP 通但不响应），此时必须中止请求并走浏览器下载回退，不能让接管
// 流程无限等待、下载永久停在暂停状态。
export const DEFAULT_TIMEOUT_MS = 6_000;
// 媒体探测运行 yt-dlp 分析，慢站点可达数十秒。
export const PROBE_TIMEOUT_MS = 60_000;
// health 二次确认（401 清令牌前）与在线探测用短超时。
export const HEALTH_TIMEOUT_MS = 2_500;

/// 带超时的 fetch。`options.timeoutMs` 控制中止时间（默认 DEFAULT_TIMEOUT_MS），
/// 其余选项原样透传给 fetch。超时抛出 AbortError（DOMException）。
async function fetchWithTimeout(url, options = {}) {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, ...rest } = options;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...rest, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

export async function compatFetch(path, options = {}) {
  const url = path.startsWith("http") ? path : `${API}${path}`;
  try {
    return await fetchWithTimeout(url, options);
  } catch (error) {
    if (url.includes("127.0.0.1")) {
      return await fetchWithTimeout(url.replace("127.0.0.1", "localhost"), options);
    }
    throw error;
  }
}

export async function signature(token, timestamp, body) {
  const encoder = new TextEncoder();
  const keyBytes = await crypto.subtle.digest("SHA-256", encoder.encode(token));
  const key = await crypto.subtle.importKey("raw", keyBytes, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const signed = await crypto.subtle.sign("HMAC", key, encoder.encode(`${timestamp}\n${body}`));
  return bytesToHex(new Uint8Array(signed));
}

/// 401 清令牌前先做 health 二次确认（roadmap F-13）。
///
/// 直接清令牌的问题：401 也可能来自桌面端临时异常或版本不一致，此时清掉
/// 令牌会迫使用户重新配对。只有 health 探测返回 2xx（桥接活着、确实拒绝了
/// 我们的签名）才认定令牌失效；health 本身不可达时保留令牌，下次再试。
async function confirmTokenInvalid() {
  try {
    const response = await compatFetch("/v1/health", { timeoutMs: HEALTH_TIMEOUT_MS });
    return response.ok;
  } catch {
    return false;
  }
}

async function handleUnauthorized() {
  if (await confirmTokenInvalid()) {
    await chrome.storage.local.remove("bridgeToken").catch(() => {});
    return true; // 令牌确认失效，已清除。
  }
  return false; // 桥接状态不明，保留令牌待下次验证。
}

async function signedHeaders(token, timestamp, body) {
  return {
    "Content-Type": "application/json",
    "X-Luma-Extension": chrome.runtime.id,
    "X-Luma-Timestamp": timestamp,
    "X-Luma-Signature": await signature(token, timestamp, body),
    "Origin": `chrome-extension://${chrome.runtime.id}`,
  };
}

export async function signedFetch(path, payload, options = {}) {
  const { bridgeToken } = await chrome.storage.local.get("bridgeToken");
  if (!bridgeToken) throw new Error("尚未与桌面端配对");
  const body = JSON.stringify(payload); const timestamp = Date.now().toString();
  const res = await compatFetch(path, {
    ...options,
    method: "POST",
    headers: await signedHeaders(bridgeToken, timestamp, body),
    body
  });
  if (res.status === 401) {
    await handleUnauthorized();
  }
  return res;
}

// GET 请求的签名版本（SubTask 13.1）。
// GET 没有 body，签名覆盖 `timestamp\n`（空 body），与 Rust 端 `authorize(&[], ...)`
// 即 `mac.update(timestamp); mac.update(b"\n"); mac.update(&[])` 一致。
export async function signedGet(path, options = {}) {
  const { bridgeToken } = await chrome.storage.local.get("bridgeToken");
  if (!bridgeToken) throw new Error("尚未与桌面端配对");
  const timestamp = Date.now().toString();
  const sig = await signature(bridgeToken, timestamp, "");
  const res = await compatFetch(path, {
    ...options,
    method: "GET",
    headers: {
      "X-Luma-Extension": chrome.runtime.id,
      "X-Luma-Timestamp": timestamp,
      "X-Luma-Signature": sig,
      "Origin": `chrome-extension://${chrome.runtime.id}`,
    },
  });
  if (res.status === 401) {
    await handleUnauthorized();
  }
  return res;
}

/// 唤起并聚焦桌面端主窗口（POST /v1/focus）。
///
/// 用于系统通知点击闭环：扩展通知（任务已添加/离线提示等）被点击时调用。
/// 桌面端离线或未配对时返回 false，调用方自行决定后续提示。
export async function focusDesktop() {
  try {
    const response = await signedFetch("/v1/focus", {}, { timeoutMs: HEALTH_TIMEOUT_MS });
    return response.ok;
  } catch {
    return false;
  }
}
