export const API = "http://127.0.0.1:17433";
const bytesToHex = (bytes) => [...bytes].map((value) => value.toString(16).padStart(2, "0")).join("");

export async function compatFetch(path, options = {}) {
  const url = path.startsWith("http") ? path : `${API}${path}`;
  try {
    return await fetch(url, options);
  } catch (error) {
    if (url.includes("127.0.0.1")) {
      return await fetch(url.replace("127.0.0.1", "localhost"), options);
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

export async function signedFetch(path, payload) {
  const { bridgeToken } = await chrome.storage.local.get("bridgeToken");
  if (!bridgeToken) throw new Error("尚未与桌面端配对");
  const body = JSON.stringify(payload); const timestamp = Date.now().toString();
  const res = await compatFetch(path, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Luma-Extension": chrome.runtime.id,
      "X-Luma-Timestamp": timestamp,
      "X-Luma-Signature": await signature(bridgeToken, timestamp, body),
      "Origin": `chrome-extension://${chrome.runtime.id}`,
    },
    body
  });
  if (res.status === 401) {
    await chrome.storage.local.remove("bridgeToken").catch(() => {});
  }
  return res;
}

// GET 请求的签名版本（SubTask 13.1）。
// GET 没有 body，签名覆盖 `timestamp\n`（空 body），与 Rust 端 `authorize(&[], ...)`
// 即 `mac.update(timestamp); mac.update(b"\n"); mac.update(&[])` 一致。
export async function signedGet(path) {
  const { bridgeToken } = await chrome.storage.local.get("bridgeToken");
  if (!bridgeToken) throw new Error("尚未与桌面端配对");
  const timestamp = Date.now().toString();
  const sig = await signature(bridgeToken, timestamp, "");
  const res = await compatFetch(path, {
    method: "GET",
    headers: {
      "X-Luma-Extension": chrome.runtime.id,
      "X-Luma-Timestamp": timestamp,
      "X-Luma-Signature": sig,
      "Origin": `chrome-extension://${chrome.runtime.id}`,
    },
  });
  if (res.status === 401) {
    await chrome.storage.local.remove("bridgeToken").catch(() => {});
  }
  return res;
}
