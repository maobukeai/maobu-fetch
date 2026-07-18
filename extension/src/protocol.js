export const API = "http://127.0.0.1:17433";
const bytesToHex = (bytes) => [...bytes].map((value) => value.toString(16).padStart(2, "0")).join("");

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
  return fetch(`${API}${path}`, { method: "POST", headers: {
    "Content-Type": "application/json", "X-Luma-Extension": chrome.runtime.id,
    "X-Luma-Timestamp": timestamp, "X-Luma-Signature": await signature(bridgeToken, timestamp, body),
  }, body });
}
