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

/// 使用当前页面登录态下载。
///
/// 流程：
///   1. 调用 `cookiesApi.getAll({ url, storeId })` 获取当前页所有 Cookie
///      （支持无痕模式：传入 `cookieStoreId` 时从对应 cookie store 读取）
///   2. 拼接成 Cookie 头
///   3. 通过 `sendMessage({type:"send", url, extra:{headers:{Cookie, Referer, "User-Agent"}}})`
///      一次性传递给桌面端
///   4. 不写入 `chrome.storage.local`（不持久化）
///
/// 参数：
///   - `url`：当前页 URL（用于 cookies.getAll 和作为下载 URL）
///   - `pageUrl`：用于读取 Cookie 的源页面 URL（默认与 `url` 相同）
///   - `userAgent`：navigator.userAgent
///   - `cookiesApi`：通常是 `chrome.cookies`，测试时可注入 mock
///   - `sendMessage`：通常是 `chrome.runtime.sendMessage` 的 Promise 包装，测试时可注入 mock
///   - `cookieStoreId`：当前 tab 所在的 cookie store id（用于无痕模式）。
///     普通窗口传 `undefined` 即可，无痕窗口由 popup.js 从 `tab.cookieStoreId` 透传。
///     Chrome 无痕窗口的 Cookie 存放在独立 store，不传 storeId 时 `chrome.cookies.getAll`
///     只读主 store，无法拿到无痕窗口的 Cookie。
///
/// 返回 `{ ok, error? }`。失败时 `error` 为可读字符串，不含 Cookie 原文。
export async function sendWithCurrentPageAuth({ url, pageUrl, userAgent, cookiesApi, sendMessage, cookieStoreId }) {
  const cookieSourceUrl = pageUrl || url;
  if (!cookieSourceUrl || !/^https?:/i.test(cookieSourceUrl)) {
    return { ok: false, error: "当前页面不是 HTTP/HTTPS 页面，无法获取登录态" };
  }
  if (!url || !/^https?:/i.test(url)) {
    return { ok: false, error: "下载目标不是有效的 HTTP/HTTPS URL" };
  }
  // 显式指定 storeId 时从对应 cookie store 读取（无痕模式必需）；
  // 不指定时按默认 store 读取（普通窗口行为不变）。
  const getAllParams = { url: cookieSourceUrl };
  if (cookieStoreId) getAllParams.storeId = cookieStoreId;
  const cookies = await cookiesApi.getAll(getAllParams).catch(() => []);
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

/// 已知媒体平台域名归一化表。
/// 用于导出 cookies.txt 时把子域名归并到主域，与桌面端
/// `media_cookies.rs::normalize_cookie_domain` 保持一致。
const KNOWN_MEDIA_DOMAINS = [
  ["douyin.com", ".douyin.com"],
  ["iesdouyin.com", ".douyin.com"],
  ["tiktok.com", ".tiktok.com"],
  ["bilibili.com", ".bilibili.com"],
  ["youtube.com", ".youtube.com"],
  ["twitter.com", ".twitter.com"],
  ["x.com", ".twitter.com"],
  ["weibo.com", ".weibo.com"],
  ["weibo.cn", ".weibo.cn"],
];

function normalizeCookieDomain(domain) {
  const lower = (domain || "").toLowerCase();
  for (const [p, target] of KNOWN_MEDIA_DOMAINS) {
    if (lower === p || lower.endsWith(`.${p}`)) return target;
  }
  // 未知域名：host-only 用裸域名，含子域的保留 leading dot
  return lower.startsWith(".") ? lower : `.${lower}`;
}

/// 把 chrome.cookies.getAll 返回的 cookie 数组转换为 Netscape cookies.txt 格式文本。
///
/// 格式（与 yt-dlp `--cookies` 参数兼容，与桌面端
/// `media_cookies.rs::build_netscape_content` 一致）：
///   # Netscape HTTP Cookie File
///   domain<TAB>include_subdomains<TAB>path<TAB>secure<TAB>expiration<TAB>name<TAB>value
///
/// - `domain`：从 pageUrl 提取并归一化（如 www.youtube.com → .youtube.com）
/// - `include_subdomains`：归一化后域名带 leading dot 则为 TRUE，否则 FALSE
/// - `secure`：按 pageUrl scheme 决定
/// - `expiration`：cookie 自带的 expirationDate（秒）；会话 cookie 为 0
///
/// 不在导出内容里写入 cookie 原文以外的信息（不上传浏览历史或页面数据）。
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
  const domain = normalizeCookieDomain(host);
  const includeSubdomains = domain.startsWith(".") ? "TRUE" : "FALSE";
  const secureStr = isHttps ? "TRUE" : "FALSE";
  const lines = ["# Netscape HTTP Cookie File"];
  for (const c of cookies) {
    if (!c || typeof c.name !== "string" || c.name === "") continue;
    const expiration = typeof c.expirationDate === "number" && c.expirationDate > 0
      ? Math.floor(c.expirationDate)
      : 0;
    const path = typeof c.path === "string" && c.path !== "" ? c.path : "/";
    lines.push(
      [domain, includeSubdomains, path, secureStr, String(expiration), c.name, c.value ?? ""].join("\t")
    );
  }
  return lines.join("\n") + "\n";
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
  triggerDownload,
}) {
  if (!pageUrl || !/^https?:/i.test(pageUrl)) {
    return { ok: false, error: "当前页面不是 HTTP/HTTPS 页面，无法获取登录态" };
  }
  const getAllParams = { url: pageUrl };
  if (cookieStoreId) getAllParams.storeId = cookieStoreId;
  const cookies = await cookiesApi.getAll(getAllParams).catch(() => []);
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
