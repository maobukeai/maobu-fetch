const TRACKING_NAMES = new Set([
  "fbclid", "gclid", "dclid", "msclkid", "mc_cid", "mc_eid", "igshid",
]);
const SENSITIVE_NAME = /(token|signature|sig|expires|auth|key|credential|policy|x-amz-)/i;

export function normalizeHost(value) {
  const candidate = String(value || "").trim().toLowerCase().replace(/^\*\./, "").replace(/^\./, "").replace(/\.$/, "");
  if (!candidate || /[\s/@?#]/.test(candidate)) throw new Error("域名格式无效");
  const parsed = new URL(`http://${candidate}`);
  if (parsed.username || parsed.password || parsed.port || parsed.pathname !== "/") throw new Error("域名格式无效");
  return parsed.hostname.toLowerCase();
}

export function normalizeExtension(value) {
  const extension = String(value || "").trim().toLowerCase().replace(/^\./, "");
  if (!/^[a-z0-9]{1,12}$/.test(extension)) throw new Error("文件类型格式无效");
  return extension;
}

export function parseRules(value, normalize) {
  const values = [];
  const invalid = [];
  for (const item of String(value || "").split(/[\n,]+/)) {
    if (!item.trim()) continue;
    try { values.push(normalize(item)); } catch { invalid.push(item.trim()); }
  }
  return { values: [...new Set(values)], invalid };
}

export function cleanTrackingUrl(value) {
  let url;
  try { url = new URL(value); } catch { return value; }
  if (!["http:", "https:"].includes(url.protocol)) return value;
  const names = [...url.searchParams.keys()];
  if (names.some((name) => SENSITIVE_NAME.test(name))) return value;
  let changed = false;
  for (const name of names) {
    if (/^utm_/i.test(name) || TRACKING_NAMES.has(name.toLowerCase())) {
      url.searchParams.delete(name);
      changed = true;
    }
  }
  return changed ? url.toString() : value;
}

export async function requestPageWithTrackingFallback(request, originalUrl) {
  const cleanedUrl = cleanTrackingUrl(originalUrl);
  if (cleanedUrl === originalUrl) return request(originalUrl);
  try {
    const response = await request(cleanedUrl);
    if (response.ok) return response;
  } catch { /* 使用原始页面地址重试。 */ }
  return request(originalUrl);
}
