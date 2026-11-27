// Chromium CookieStore.tabIds 是普通/无痕隔离的权威，不读取 Firefox-only Tab.cookieStoreId。
export async function resolveCookieStore(cookiesApi, { tab, tabId, storeId, incognito } = {}) {
  if (typeof storeId === "string" && storeId) return storeId;
  if (!cookiesApi?.getAllCookieStores) return null;
  let stores;
  try {
    stores = await cookiesApi.getAllCookieStores();
  } catch { return null; }
  const id = tab?.id ?? tabId;
  if (Number.isInteger(id) && id >= 0) {
    const matched = stores.find((store) => store.tabIds?.includes(id))?.id;
    if (matched) return matched;
  }
  // 仅当浏览器明确标明是非无痕（incognito === false）且原标签页已关闭/无 tabId 时，
  // 回退到默认普通 store（id === "0"）；无痕或身份不明场景绝不猜测普通存储。
  if (incognito === false && tab?.incognito !== true) {
    return stores.find((store) => store.id === "0")?.id ?? null;
  }
  return null;
}

function cookieMatchesUrl(cookie, target) {
  if (!cookie?.name || /[\r\n;]/.test(cookie.name) || /[\r\n]/.test(String(cookie.value || ""))) return false;
  if (cookie.secure && target.protocol !== "https:") return false;
  if (cookie.domain) {
    const domain = cookie.domain.replace(/^\./, "").toLowerCase();
    if (target.hostname !== domain && (cookie.hostOnly || !target.hostname.endsWith(`.${domain}`))) return false;
  }
  const path = cookie.path || "/";
  return target.pathname === path || (target.pathname.startsWith(path)
    && (path.endsWith("/") || target.pathname[path.length] === "/"));
}

/** 仅提取最终目标 URL 的 Cookie；无法确认 store 时不猜测普通窗口身份。 */
export async function readTargetCookies(cookiesApi, url, context = {}) {
  let target;
  try { target = new URL(url); } catch { return []; }
  if (!/^https?:$/.test(target.protocol) || !cookiesApi?.getAll) return [];
  const storeId = await resolveCookieStore(cookiesApi, context);
  if (storeId === null) return [];
  const params = { url: target.href, storeId };
  let cookies;
  try { cookies = await cookiesApi.getAll(params); } catch { return []; }
  const id = context.tab?.id ?? context.tabId;
  // Chrome 132+ 能给出实际 frame partition；旧版本仅传递未分区 Cookie。
  if (Number.isInteger(id) && cookiesApi.getPartitionKey) {
    try {
      const { partitionKey } = await cookiesApi.getPartitionKey({ tabId: id, frameId: context.frameId ?? 0 });
      if (partitionKey?.topLevelSite) {
        const partitioned = await cookiesApi.getAll({ ...params, partitionKey });
        cookies = [...(cookies || []), ...(partitioned || [])];
      }
    } catch { /* 不得在无法确认分区时回退到其他分区。 */ }
  }
  const seen = new Set();
  return (cookies || []).filter((cookie) => {
    if (!cookieMatchesUrl(cookie, target)) return false;
    const key = JSON.stringify([cookie.name, cookie.domain, cookie.path, cookie.partitionKey]);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  }).sort((a, b) => (b.path || "/").length - (a.path || "/").length);
}
