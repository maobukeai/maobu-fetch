import { readTargetCookies } from "./cookie-context.js";
// SubTask 45.2～45.5：使用当前页面登录态下载。
//
// 把"获取当前页 Cookie → 拼接成 Cookie 头 → 通过本地桥一次性传递给桌面端"
// 抽成独立可测试的辅助函数。popup.js 顶层有大量副作用，难以直接单元测试；
// 把这部分逻辑独立出来便于在 bridge_tasks.test.js 中以模拟 Chrome API 验证。
//
// 合规要点（AGENTS.md §3、§5）：
//   - 仅传递当前页 Cookie，不上传浏览历史或页面数据
//   - Cookie 仅一次性传递给桌面端，不持久化在扩展 storage 中
//   - 认证信息不得写入日志、错误历史或前端调试输出

/// 把 chrome.cookies.getAll 返回的 cookie 数组拼接成 "name1=value1; name2=value2" 格式。
/// 空值或缺少 name 的条目会被跳过。
export function buildCookieHeader(cookies) {
  if (!Array.isArray(cookies)) return "";
  return cookies
    .map((c) => (c && typeof c.name === "string" && c.name !== "" ? `${c.name}=${c.value ?? ""}` : ""))
    .filter((segment) => segment !== "")
    .join("; ");
}

/// 用户主动发起的一次性认证下载。按目标 URL 和真实 tab/store 读取 Cookie，
/// pageUrl 仅用作 Referer；不同域的页面登录态不会借给目标。
/// 显式 cookieStoreId 仅为旧内部调用兼容保留，实际 popup 传 tab 并解析 CookieStore.tabIds。
export async function sendWithCurrentPageAuth({ url, pageUrl, userAgent, cookiesApi, sendMessage, cookieStoreId, tab }) {
  const cookieSourceUrl = pageUrl || url;
  if (!cookieSourceUrl || !/^https?:/i.test(cookieSourceUrl)) {
    return { ok: false, error: "当前页面不是 HTTP/HTTPS 页面，无法获取登录态" };
  }
  if (!url || !/^https?:/i.test(url)) {
    return { ok: false, error: "下载目标不是有效的 HTTP/HTTPS URL" };
  }
  const cookies = await readTargetCookies(cookiesApi, url, { tab, storeId: cookieStoreId });
  const cookieHeader = buildCookieHeader(cookies);
  if (!cookieHeader) {
    return { ok: false, error: "当前页面没有可用的 Cookie" };
  }
  // 仅传递 Cookie/Referer/User-Agent；不上传浏览历史或页面数据（AGENTS.md §5）。
  const response = await sendMessage({
    type: "send",
    url,
    fileName: undefined,
    extra: {
      headers: {
        Cookie: cookieHeader,
        Referer: cookieSourceUrl,
        "User-Agent": userAgent || "",
      },
    },
  });
  if (response?.ok) {
    return { ok: true };
  }
  return { ok: false, error: response?.error || "请检查配对状态" };
}

/// Netscape 文件不能表示 CHIPS 分区键；只导出未分区 Cookie，保留其原始域/path/secure/hostOnly。
export function buildNetscapeCookiesText(cookies, pageUrl) {
  if (!Array.isArray(cookies) || cookies.length === 0) return "";
  let host = "";
  let isHttps = false;
  try {
    const u = new URL(pageUrl);
    host = u.hostname;
    isHttps = u.protocol === "https:";
  } catch {
    return "";
  }
  const lines = ["# Netscape HTTP Cookie File"];
  for (const c of cookies) {
    if (!c || typeof c.name !== "string" || c.name === "" || c.partitionKey) continue;
    const rawDomain = String(c.domain || host).toLowerCase();
    const include = c.hostOnly === false || (c.hostOnly !== true && rawDomain.startsWith("."));
    const domain = (include ? "." : "") + rawDomain.replace(/^\./, "");
    const includeSubdomains = include ? "TRUE" : "FALSE";
    const secureStr = (typeof c.secure === "boolean" ? c.secure : isHttps) ? "TRUE" : "FALSE";
    const expiration = typeof c.expirationDate === "number" && c.expirationDate > 0
      ? Math.floor(c.expirationDate)
      : 0;
    const path = typeof c.path === "string" && c.path !== "" ? c.path : "/";
    const fields = [domain, includeSubdomains, path, secureStr, String(expiration), c.name, c.value ?? ""];
    if (fields.some((field) => /[\t\r\n]/.test(String(field)))) continue;
    lines.push(fields.join("\t"));
  }
  return lines.length > 1 ? lines.join("\n") + "\n" : "";
}

/// 导出当前页面 Cookie 为 cookies.txt 文件。
///
/// 流程：
///   1. 调用 `cookiesApi.getAll({ url, storeId })` 获取当前页所有 Cookie
///      （支持无痕模式：传入 `cookieStoreId` 时从对应 cookie store 读取）
///   2. 转换为 Netscape cookies.txt 格式文本
///   3. 通过 `triggerDownload` 触发浏览器下载（默认是 `chrome.runtime.download`
///      不可用时的回退实现：URL.createObjectURL + <a download>）
///
/// 不写入 `chrome.storage.local`，不通过网络发送给桌面端。
/// 用户需手动把导出的文件导入到「设置 → 媒体凭证」。
///
/// 返回 `{ ok, error?, fileName? }`。失败时 `error` 为可读字符串，不含 Cookie 原文。
export async function exportCurrentPageCookies({
  pageUrl,
  cookiesApi,
  cookieStoreId,
  tab,
  triggerDownload,
}) {
  if (!pageUrl || !/^https?:/i.test(pageUrl)) {
    return { ok: false, error: "当前页面不是 HTTP/HTTPS 页面，无法获取登录态" };
  }
  const cookies = await readTargetCookies(cookiesApi, pageUrl, { tab, storeId: cookieStoreId });
  if (!cookies || cookies.length === 0) {
    return { ok: false, error: "当前页面没有可用的 Cookie" };
  }
  const content = buildNetscapeCookiesText(cookies, pageUrl);
  if (!content) {
    return { ok: false, error: "Cookie 解析失败" };
  }
  let host = "site";
  try {
    host = new URL(pageUrl).hostname.replace(/^www\./, "");
  } catch {}
  const fileName = `cookies_${host}.txt`;
  if (typeof triggerDownload === "function") {
    await triggerDownload(content, fileName);
  }
  return { ok: true, fileName };
}
