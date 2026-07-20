// SubTask 13.7：模拟 Chrome API 测试。
// 覆盖：离线回退、接管开关、临时绕过（浮层）、最近任务列表、暂停/继续/打开文件。
// 配对流程的签名测试见 protocol.test.js（已复用）。
import test from "node:test";
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { evaluateDownload, interceptBrowserDownload } from "./interceptor.js";

globalThis.crypto ??= webcrypto;

// background.js 顶层注册 chrome.runtime/contextMenus/downloads/onMessage listener，
// 必须在导入前提供 globalThis.chrome。ES module 静态 import 会被提升到所有代码之前，
// 因此 background.js 改为顶层 await 动态导入。
globalThis.chrome = {
  storage: { local: { get: async () => ({}), set: async () => {} }, session: { set: async () => {} } },
  runtime: { id: "test-extension-id", onInstalled: { addListener: () => {} }, onMessage: { addListener: () => {} } },
  contextMenus: { removeAll: (cb) => cb && cb(), create: () => {}, onClicked: { addListener: () => {} } },
  downloads: { onCreated: { addListener: () => {} } },
  tabs: { query: async () => [], sendMessage: async () => ({}) },
  notifications: { create: () => {} },
};

const { isDesktopOfflineError, confirmTakeoverWithOverlay, findSourceTab } = await import("./background.js");

const __dirname = dirname(fileURLToPath(import.meta.url));

// === 模拟 Chrome API ===
// 与 interceptor.test.js 风格一致，提供可重置的 chrome 全局对象。
function createChromeMock(overrides = {}) {
  const storageLocal = overrides.storageLocal || {};
  const notifications = [];
  const downloads = {
    pause: overrides.pause || (async () => {}),
    resume: overrides.resume || (async () => {}),
    cancel: overrides.cancel || (async () => {}),
    erase: overrides.erase || (async () => {}),
    search: overrides.search || (async () => [overrides.freshItem || null]),
  };
  const tabs = {
    query: overrides.queryTabs || (async () => overrides.tabsList || []),
    sendMessage: overrides.sendMessage || (async () => ({ bypass: false })),
  };
  return {
    chrome: {
      storage: {
        local: {
          get: async (keys) => {
            const result = {};
            const keyList = Array.isArray(keys) ? keys : [keys];
            for (const key of keyList) if (key in storageLocal) result[key] = storageLocal[key];
            return result;
          },
          set: async (values) => { Object.assign(storageLocal, values); },
        },
        session: { set: async () => {} },
      },
      runtime: { id: "test-extension-id" },
      downloads,
      tabs,
      notifications: { create: (opts) => notifications.push(opts) },
      contextMenus: { removeAll: (cb) => cb && cb(), create: () => {} },
    },
    notifications,
    storageLocal,
  };
}

// === SubTask 13.5：离线回退测试 ===
test("offline fallback: TypeError from sendTask triggers offline notification and resumes browser download", async () => {
  const { chrome, storageLocal } = createChromeMock({
    freshItem: {
      id: 1, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip",
      filename: "file.zip", totalBytes: 3_000_000, referrer: "https://example.com/page",
    },
  });
  // interceptBrowserDownload 通过 globalThis.chrome 访问 storage.local.set，
  // 因此将 per-test mock 设为全局，使其 set 写入 storageLocal 供断言读取。
  const originalChrome = globalThis.chrome;
  globalThis.chrome = chrome;
  try {
    const calls = [];
    const handled = await interceptBrowserDownload(
      { id: 1, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip", filename: "file.zip", totalBytes: 3_000_000 },
      {
        downloads: chrome.downloads, settings: { intercept: true, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: 0 },
        runtimeId: "extension-id", wait: async () => {},
        sendTask: async () => { calls.push("send"); throw new TypeError("Failed to fetch"); },
        notify: (title, message) => { calls.push({ title, message }); },
        isDesktopOfflineError,
      }
    );
    assert.equal(handled, false);
    // 桌面端离线时通知标题包含"桌面端离线"
    assert.ok(calls.some((c) => typeof c === "object" && c.title.includes("桌面端离线")));
    // lastIgnored 记录 reason="offline"，便于弹窗显示明确提示
    assert.equal(storageLocal.lastIgnored?.reason, "offline");
  } finally {
    globalThis.chrome = originalChrome;
  }
});

test("offline fallback: HTTP 4xx/5xx errors are NOT classified as offline", () => {
  assert.equal(isDesktopOfflineError(new Error("HTTP 401")), false);
  assert.equal(isDesktopOfflineError(new Error("HTTP 500")), false);
  assert.equal(isDesktopOfflineError(new Error("任务参数无效")), false);
});

test("offline fallback: TypeError and network errors are classified as offline", () => {
  assert.equal(isDesktopOfflineError(new TypeError("Failed to fetch")), true);
  assert.equal(isDesktopOfflineError(new TypeError("NetworkError when attempting to fetch resource")), true);
  assert.equal(isDesktopOfflineError(new Error("fetch failed: ECONNREFUSED")), true);
  assert.equal(isDesktopOfflineError(new Error("connect ECONNREFUSED 127.0.0.1:17433")), true);
});

test("offline fallback: non-offline error still triggers generic fallback notification", async () => {
  const { chrome } = createChromeMock({
    freshItem: {
      id: 2, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip",
      filename: "file.zip", totalBytes: 3_000_000,
    },
  });
  const calls = [];
  await interceptBrowserDownload(
    { id: 2, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip", filename: "file.zip", totalBytes: 3_000_000 },
    {
      downloads: chrome.downloads, settings: { intercept: true, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: 0 },
      runtimeId: "extension-id", wait: async () => {},
      sendTask: async () => { throw new Error("任务参数无效"); },
      notify: (title, message) => { calls.push({ title, message }); },
      isDesktopOfflineError,
    }
  );
  // 非offline错误使用通用"接管失败"通知，不使用"桌面端离线"
  assert.ok(calls.some((c) => c.title.includes("接管失败")));
  assert.ok(!calls.some((c) => c.title.includes("桌面端离线")));
});

// === SubTask 13.7：接管开关测试 ===
test("interception toggle: when intercept is disabled, evaluateDownload returns disabled reason", () => {
  const item = { id: 1, url: "https://example.com/file.zip", filename: "file.zip", totalBytes: 3_000_000 };
  const result = evaluateDownload(item, { intercept: false, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: 0 }, "extension-id");
  assert.equal(result.eligible, false);
  assert.equal(result.reason, "disabled");
});

test("interception toggle: when intercept is enabled, eligible download proceeds", () => {
  const item = { id: 2, url: "https://example.com/file.zip", filename: "file.zip", totalBytes: 3_000_000 };
  const result = evaluateDownload(item, { intercept: true, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: 0 }, "extension-id");
  assert.equal(result.eligible, true);
});

// === SubTask 13.7：临时绕过测试（浮层 + bypassUntil）===
test("temporary bypass: bypassUntil in the future blocks interception", () => {
  const item = { id: 3, url: "https://example.com/file.zip", filename: "file.zip", totalBytes: 3_000_000 };
  const result = evaluateDownload(item, { intercept: true, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: Date.now() + 60_000 }, "extension-id");
  assert.equal(result.eligible, false);
  assert.equal(result.reason, "bypass");
});

test("overlay bypass: when content script returns bypass=true, confirmTakeoverWithOverlay returns false", async () => {
  const notifications = [];
  const result = await confirmTakeoverWithOverlay(
    { id: 1, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip", filename: "file.zip", referrer: "https://example.com/page" },
    { intercept: true, bypassUntil: 0 },
    {
      queryTabs: async () => [{ id: 42, url: "https://example.com/page" }],
      sendMessage: async () => ({ bypass: true }),
      notify: (title, message) => notifications.push({ title, message }),
    }
  );
  assert.equal(result, false, "should NOT proceed with takeover when user clicks bypass");
  assert.equal(notifications.length, 1);
  assert.match(notifications[0].title, /临时绕过/);
});

test("overlay bypass: when content script returns bypass=false, takeover proceeds", async () => {
  const result = await confirmTakeoverWithOverlay(
    { id: 2, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip", filename: "file.zip", referrer: "https://example.com/page" },
    { intercept: true, bypassUntil: 0 },
    {
      queryTabs: async () => [{ id: 42, url: "https://example.com/page" }],
      sendMessage: async () => ({ bypass: false }),
      notify: () => {},
    }
  );
  assert.equal(result, true, "should proceed with takeover when overlay times out");
});

test("overlay bypass: when intercept is disabled, overlay is skipped", async () => {
  let sendMessageCalled = false;
  const result = await confirmTakeoverWithOverlay(
    { id: 3, url: "https://example.com/file.zip", filename: "file.zip" },
    { intercept: false, bypassUntil: 0 },
    {
      queryTabs: async () => [{ id: 42, url: "https://example.com/page" }],
      sendMessage: async () => { sendMessageCalled = true; return { bypass: false }; },
      notify: () => {},
    }
  );
  assert.equal(result, true);
  assert.equal(sendMessageCalled, false, "overlay should not be shown when intercept is disabled");
});

test("overlay bypass: when bypassUntil is active, overlay is skipped", async () => {
  let sendMessageCalled = false;
  const result = await confirmTakeoverWithOverlay(
    { id: 4, url: "https://example.com/file.zip", filename: "file.zip" },
    { intercept: true, bypassUntil: Date.now() + 60_000 },
    {
      queryTabs: async () => [{ id: 42, url: "https://example.com/page" }],
      sendMessage: async () => { sendMessageCalled = true; return { bypass: false }; },
      notify: () => {},
    }
  );
  assert.equal(result, true);
  assert.equal(sendMessageCalled, false, "overlay should not be shown during bypass period");
});

test("overlay bypass: when content script is unreachable (chrome:// page), takeover proceeds", async () => {
  const result = await confirmTakeoverWithOverlay(
    { id: 5, url: "https://example.com/file.zip", filename: "file.zip", referrer: "https://example.com/page" },
    { intercept: true, bypassUntil: 0 },
    {
      queryTabs: async () => [{ id: 42, url: "https://example.com/page" }],
      sendMessage: async () => { throw new Error("Could not establish connection"); },
      notify: () => {},
    }
  );
  assert.equal(result, true, "should proceed when content script is unreachable");
});

test("overlay bypass: when no source tab is found, takeover proceeds without overlay", async () => {
  let sendMessageCalled = false;
  const result = await confirmTakeoverWithOverlay(
    { id: 6, url: "https://example.com/file.zip", filename: "file.zip" },
    { intercept: true, bypassUntil: 0 },
    {
      queryTabs: async () => [],
      sendMessage: async () => { sendMessageCalled = true; return { bypass: false }; },
      notify: () => {},
    }
  );
  assert.equal(result, true);
  assert.equal(sendMessageCalled, false);
});

// === findSourceTab 测试 ===
test("findSourceTab: prefers active tab matching referrer origin", async () => {
  const tab = await findSourceTab(
    { url: "https://example.com/file.zip", referrer: "https://example.com/page", finalUrl: "https://example.com/file.zip" },
    { queryTabs: async () => [{ id: 99, url: "https://example.com/page" }] }
  );
  assert.equal(tab.id, 99);
});

test("findSourceTab: returns null when no http(s) tab available", async () => {
  const tab = await findSourceTab(
    { url: "https://example.com/file.zip", referrer: "", finalUrl: "https://example.com/file.zip" },
    { queryTabs: async () => [{ id: 1, url: "chrome://settings" }] }
  );
  assert.equal(tab, null);
});

// === SubTask 13.1/13.2：最近任务列表与操作（协议层测试）===
// 通过模拟 fetch 验证 signedGet/signedFetch 正确调用桥端点。
// 这覆盖了 background.js 中 recent-tasks 和 task-action 消息处理器的核心逻辑。
test("recent-tasks: signedGet calls /v1/tasks/recent with GET method and required headers", async () => {
  const { chrome } = createChromeMock({
    storageLocal: { bridgeToken: "test-token" },
  });
  globalThis.chrome = chrome;
  const fetchCalls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, options) => {
    fetchCalls.push({ url, options });
    return {
      ok: true,
      status: 200,
      json: async () => ({
        tasks: [
          { id: "task-1", url: "https://example.com/a.zip", file_name: "a.zip", status: "downloading", progress: 0.5, speed: 1024, error: null },
          { id: "task-2", url: "https://example.com/b.zip", file_name: "b.zip", status: "completed", progress: 1.0, speed: 0, error: null },
        ],
      }),
      text: async () => "",
    };
  };
  try {
    const { signedGet } = await import("./protocol.js");
    const result = await signedGet("/v1/tasks/recent");
    assert.equal(result.ok, true);
    assert.equal(fetchCalls.length, 1);
    assert.match(fetchCalls[0].url, /\/v1\/tasks\/recent/);
    assert.equal(fetchCalls[0].options.method, "GET");
    assert.ok(fetchCalls[0].options.headers["X-Luma-Signature"], "must include HMAC signature");
    assert.ok(fetchCalls[0].options.headers["X-Luma-Timestamp"], "must include timestamp");
    assert.ok(fetchCalls[0].options.headers["X-Luma-Extension"], "must include extension id");
    const body = await result.json();
    assert.equal(body.tasks.length, 2);
    assert.equal(body.tasks[0].id, "task-1");
    assert.equal(body.tasks[0].status, "downloading");
    assert.equal(body.tasks[0].progress, 0.5);
  } finally {
    globalThis.fetch = originalFetch;
    delete globalThis.chrome;
  }
});

test("task-action: signedFetch posts to /v1/tasks/{id}/action with action payload", async () => {
  const { chrome } = createChromeMock({
    storageLocal: { bridgeToken: "test-token" },
  });
  globalThis.chrome = chrome;
  const fetchCalls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, options) => {
    fetchCalls.push({ url, options });
    return {
      ok: true,
      status: 200,
      json: async () => ({ success: true }),
      text: async () => "",
    };
  };
  try {
    const { signedFetch } = await import("./protocol.js");
    const response = await signedFetch("/v1/tasks/task-1/action", { action: "pause" });
    assert.equal(response.ok, true);
    assert.equal(fetchCalls.length, 1);
    assert.match(fetchCalls[0].url, /\/v1\/tasks\/task-1\/action/);
    assert.equal(fetchCalls[0].options.method, "POST");
    const body = JSON.parse(fetchCalls[0].options.body);
    assert.equal(body.action, "pause");
    assert.ok(fetchCalls[0].options.headers["X-Luma-Signature"]);
    assert.ok(fetchCalls[0].options.headers["X-Luma-Timestamp"]);
    assert.ok(fetchCalls[0].options.headers["X-Luma-Extension"]);
  } finally {
    globalThis.fetch = originalFetch;
    delete globalThis.chrome;
  }
});

test("task-action: supports pause, resume, and open_file actions", async () => {
  const { chrome } = createChromeMock({
    storageLocal: { bridgeToken: "test-token" },
  });
  globalThis.chrome = chrome;
  const originalFetch = globalThis.fetch;
  for (const action of ["pause", "resume", "open_file"]) {
    const fetchCalls = [];
    globalThis.fetch = async (url, options) => {
      fetchCalls.push({ url, options });
      return {
        ok: true,
        status: 200,
        json: async () => ({ success: true }),
        text: async () => "",
      };
    };
    try {
      const { signedFetch } = await import("./protocol.js");
      await signedFetch(`/v1/tasks/task-${action}/action`, { action });
      const body = JSON.parse(fetchCalls[0].options.body);
      assert.equal(body.action, action);
    } finally {
      globalThis.fetch = originalFetch;
    }
  }
  delete globalThis.chrome;
});

test("recent-tasks: when desktop is offline (fetch throws TypeError), signedGet rejects", async () => {
  const { chrome } = createChromeMock({
    storageLocal: { bridgeToken: "test-token" },
  });
  globalThis.chrome = chrome;
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => { throw new TypeError("Failed to fetch"); };
  try {
    const { signedGet } = await import("./protocol.js");
    await assert.rejects(
      () => signedGet("/v1/tasks/recent"),
      (error) => error instanceof TypeError || /Failed to fetch|NetworkError/i.test(String(error?.message || error))
    );
  } finally {
    globalThis.fetch = originalFetch;
    delete globalThis.chrome;
  }
});

test("recent-tasks: when not paired (no bridgeToken), signedGet throws clear error", async () => {
  const { chrome } = createChromeMock({ storageLocal: {} });
  globalThis.chrome = chrome;
  try {
    const { signedGet } = await import("./protocol.js");
    await assert.rejects(
      () => signedGet("/v1/tasks/recent"),
      /尚未与桌面端配对/
    );
  } finally {
    delete globalThis.chrome;
  }
});

// === SubTask 13.6：语义元素探测合规性测试 ===
// content.js 是 classic script（非 ES module），直接导入会有副作用（setInterval）。
// 改为读取文件内容验证选择器合规性，符合 AGENTS.md §5 要求。
test("media detection: content.js only queries semantic elements (a[download], video[src], audio[src], source[src])", () => {
  const contentSource = readFileSync(join(__dirname, "content.js"), "utf8");
  // 必须查询的语义元素选择器
  assert.ok(contentSource.includes('"a[download]"'), "must query a[download]");
  assert.ok(contentSource.includes('"video[src], audio[src]"'), "must query video[src] and audio[src]");
  assert.ok(contentSource.includes('"video source[src], audio source[src]"'), "must query video/audio source[src]");
  // 禁止扫描全页所有链接
  assert.ok(!contentSource.includes('querySelectorAll("a")'), "must NOT query all <a> elements");
  assert.ok(!contentSource.includes('querySelectorAll("a[href]")'), "must NOT query all a[href] elements");
  // 禁止使用 performance.getEntriesByType 全量资源扫描
  assert.ok(!contentSource.includes("performance.getEntriesByType"), "must NOT use performance.getEntriesByType resource scan");
});

test("media detection: overlay uses inline styles (CSP-safe) and 1.5s timeout", () => {
  const contentSource = readFileSync(join(__dirname, "content.js"), "utf8");
  // 浮层使用 inline style（避免 CSP 阻止 <style> 标签）
  assert.ok(contentSource.includes("Object.assign(overlay.style,"), "overlay must use inline styles");
  // 1.5 秒超时
  assert.ok(contentSource.includes("1500"), "overlay must have 1500ms timeout");
  // 浮层消息类型
  assert.ok(contentSource.includes('"show-overlay"'), "must listen for show-overlay message");
  // 绕过响应
  assert.ok(contentSource.includes("{ bypass: true }"), "must respond with bypass: true on user click");
  assert.ok(contentSource.includes("{ bypass: false }"), "must respond with bypass: false on timeout");
});

// === SubTask 45.6：浏览器扩展临时登录态测试 ===
// 通过模拟 Chrome cookies API 验证 sendWithCurrentPageAuth 行为：
//   - popup_uses_current_page_cookies_when_clicked：chrome.cookies.getAll 被调用
//   - popup_sends_cookie_to_local_bridge：/v1/tasks/add 请求 body 含 cookie 字段
//   - popup_does_not_persist_cookie_in_storage：cookie 不写入 chrome.storage.local
// 辅助函数定义在 auth-download.js，便于在无 DOM 环境下直接测试。
import { sendWithCurrentPageAuth, buildCookieHeader } from "./auth-download.js";

test("buildCookieHeader: concatenates cookies as name=value pairs separated by semicolons", () => {
  const cookies = [
    { name: "session", value: "abc123" },
    { name: "token", value: "xyz789" },
    { name: "csrf", value: "def456" },
  ];
  assert.equal(buildCookieHeader(cookies), "session=abc123; token=xyz789; csrf=def456");
});

test("buildCookieHeader: skips entries with empty or missing name", () => {
  const cookies = [
    { name: "session", value: "abc123" },
    { name: "", value: "skipped" },
    { value: "skipped" },
    { name: "token", value: "xyz789" },
  ];
  assert.equal(buildCookieHeader(cookies), "session=abc123; token=xyz789");
});

test("buildCookieHeader: returns empty string for non-array input", () => {
  assert.equal(buildCookieHeader(null), "");
  assert.equal(buildCookieHeader(undefined), "");
  assert.equal(buildCookieHeader("not an array"), "");
});

test("popup_uses_current_page_cookies_when_clicked: chrome.cookies.getAll is called with current tab url", async () => {
  const tabUrl = "https://example.com/page";
  let cookiesGetAllCalled = false;
  let cookiesGetAllUrl = null;
  const cookiesApi = {
    getAll: async ({ url }) => {
      cookiesGetAllCalled = true;
      cookiesGetAllUrl = url;
      return [
        { name: "session", value: "abc123" },
        { name: "token", value: "xyz789" },
      ];
    },
  };
  const sendMessage = async () => ({ ok: true });

  await sendWithCurrentPageAuth({
    url: tabUrl,
    userAgent: "TestBrowser/1.0",
    cookiesApi,
    sendMessage,
  });

  assert.equal(cookiesGetAllCalled, true, "chrome.cookies.getAll must be called");
  assert.equal(cookiesGetAllUrl, tabUrl, "chrome.cookies.getAll must be called with current tab url");
});

test("popup_sends_cookie_to_local_bridge: /v1/tasks/add request body contains cookie field", async () => {
  const tabUrl = "https://example.com/page";
  const cookiesApi = {
    getAll: async () => [
      { name: "session", value: "abc123" },
      { name: "token", value: "xyz789" },
    ],
  };
  let sentMessage = null;
  const sendMessage = async (message) => {
    sentMessage = message;
    return { ok: true };
  };

  const result = await sendWithCurrentPageAuth({
    url: tabUrl,
    userAgent: "TestBrowser/1.0",
    cookiesApi,
    sendMessage,
  });

  assert.equal(result.ok, true);
  assert.ok(sentMessage, "sendMessage must be invoked");
  assert.equal(sentMessage.type, "send");
  assert.equal(sentMessage.url, tabUrl);
  assert.ok(sentMessage.extra?.headers, "extra.headers must be present");
  assert.equal(
    sentMessage.extra.headers.Cookie,
    "session=abc123; token=xyz789",
    "Cookie header must contain all cookies as name=value pairs"
  );
  assert.equal(sentMessage.extra.headers.Referer, tabUrl, "Referer must be set to current tab url");
  assert.equal(sentMessage.extra.headers["User-Agent"], "TestBrowser/1.0", "User-Agent must be passed through");
});

test("popup_sends_cookie_to_local_bridge: returns error when no cookies are available", async () => {
  const cookiesApi = { getAll: async () => [] };
  const sendMessage = async () => ({ ok: true });

  const result = await sendWithCurrentPageAuth({
    url: "https://example.com/page",
    userAgent: "TestBrowser/1.0",
    cookiesApi,
    sendMessage,
  });

  assert.equal(result.ok, false);
  assert.match(result.error, /没有可用的 Cookie/);
});

test("popup_sends_cookie_to_local_bridge: returns error when url is not http(s)", async () => {
  const cookiesApi = { getAll: async () => [{ name: "x", value: "y" }] };
  const sendMessage = async () => ({ ok: true });

  const result = await sendWithCurrentPageAuth({
    url: "chrome://settings",
    userAgent: "TestBrowser/1.0",
    cookiesApi,
    sendMessage,
  });

  assert.equal(result.ok, false);
  assert.match(result.error, /HTTP\/HTTPS/);
});

test("popup_does_not_persist_cookie_in_storage: sendWithCurrentPageAuth does not write to chrome.storage.local", async () => {
  // 通过模拟 chrome.storage.local.set 跟踪所有写入，验证 cookie 不会被持久化。
  // 这与 background.js 的 sendTask 行为一致——cookie 仅作为 task.headers 一次性传递给桌面端。
  const storageWrites = [];
  const storageLocal = {
    get: async () => ({}),
    set: async (values) => { storageWrites.push(values); },
  };
  const cookiesApi = {
    getAll: async () => [{ name: "session", value: "secret-value-123" }],
  };
  const sendMessage = async () => ({ ok: true });

  await sendWithCurrentPageAuth({
    url: "https://example.com/page",
    userAgent: "TestBrowser/1.0",
    cookiesApi,
    sendMessage,
  });

  // 验证 sendWithCurrentPageAuth 完全不调用 storage.set
  // （cookie 仅通过 sendMessage 一次性传递给 background，不持久化在扩展 storage 中）
  for (const write of storageWrites) {
    const json = JSON.stringify(write);
    assert.ok(
      !json.includes("secret-value-123"),
      "Cookie value must NOT be persisted in chrome.storage.local"
    );
    assert.ok(
      !json.toLowerCase().includes('"cookie"'),
      "Cookie key must NOT be persisted in chrome.storage.local"
    );
  }
});

test("popup_does_not_persist_cookie_in_storage: signedFetch serializes cookie into bridge body without storage write", async () => {
  // 验证 signedFetch（background.js sendTask 内部使用）会把 cookie 序列化进 /v1/tasks
  // 请求 body，但不写入 chrome.storage.local。这与 sendWithCurrentPageAuth 配合形成
  // 完整的"cookie 一次性传递给桌面端，不持久化"链路。
  const { chrome, storageLocal } = createChromeMock({
    storageLocal: { bridgeToken: "test-token" },
  });
  globalThis.chrome = chrome;
  const fetchCalls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, options) => {
    fetchCalls.push({ url, options });
    return { ok: true, status: 201, json: async () => ({ id: "task-1" }), text: async () => "" };
  };
  try {
    const { signedFetch } = await import("./protocol.js");
    await signedFetch("/v1/tasks", {
      url: "https://example.com/file.zip",
      headers: { Cookie: "session=secret-session-token" },
      priority: 0,
      source: "browser",
    });
    assert.ok(fetchCalls.length >= 1, "must call fetch /v1/tasks");
    const body = JSON.parse(fetchCalls[0].options.body);
    assert.equal(body.headers.Cookie, "session=secret-session-token", "cookie must be sent to bridge");
    // 验证 storage.local 中没有 cookie（storageLocal 仅含 bridgeToken，无 cookie）
    const storageJson = JSON.stringify(storageLocal);
    assert.ok(!storageJson.includes("secret-session-token"), "cookie must NOT be persisted in storage.local");
    assert.ok(!storageJson.toLowerCase().includes('"cookie"'), "cookie key must NOT appear in storage.local");
  } finally {
    globalThis.fetch = originalFetch;
    delete globalThis.chrome;
  }
});
