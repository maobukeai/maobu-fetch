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
///   1. 调用 `cookiesApi.getAll({ url })` 获取当前页所有 Cookie
///   2. 拼接成 Cookie 头
///   3. 通过 `sendMessage({type:"send", url, extra:{headers:{Cookie, Referer, "User-Agent"}}})`
///      一次性传递给桌面端
///   4. 不写入 `chrome.storage.local`（不持久化）
///
/// 参数：
///   - `url`：当前页 URL（用于 cookies.getAll 和作为下载 URL）
///   - `userAgent`：navigator.userAgent
///   - `cookiesApi`：通常是 `chrome.cookies`，测试时可注入 mock
///   - `sendMessage`：通常是 `chrome.runtime.sendMessage` 的 Promise 包装，测试时可注入 mock
///
/// 返回 `{ ok, error? }`。失败时 `error` 为可读字符串，不含 Cookie 原文。
export async function sendWithCurrentPageAuth({ url, pageUrl, userAgent, cookiesApi, sendMessage }) {
  const cookieSourceUrl = pageUrl || url;
  if (!cookieSourceUrl || !/^https?:/i.test(cookieSourceUrl)) {
    return { ok: false, error: "当前页面不是 HTTP/HTTPS 页面，无法获取登录态" };
  }
  if (!url || !/^https?:/i.test(url)) {
    return { ok: false, error: "下载目标不是有效的 HTTP/HTTPS URL" };
  }
  const cookies = await cookiesApi.getAll({ url: cookieSourceUrl }).catch(() => []);
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
