import test from "node:test";
import assert from "node:assert/strict";

test("Baidu credential sync reads pan.baidu.com cookies, skips incomplete tabs, and re-syncs when cookies change", async () => {
  const previousChrome = globalThis.chrome;
  const previousFetch = globalThis.fetch;
  const tabUpdatedListeners = [];
  const messageListeners = [];
  const requestedCookieUrls = [];
  const syncPayloads = [];

  let currentBaiduCookies = [{ name: "BAIDUID", value: "anon-only" }];
  const state = { bridgeToken: "synthetic-baidu-sync-token", customMediaDomains: [] };

  globalThis.chrome = {
    runtime: {
      id: "synthetic",
      onMessage: { addListener: (fn) => messageListeners.push(fn) },
      onInstalled: { addListener() {} },
    },
    storage: {
      local: {
        get: async (keys) =>
          Object.fromEntries((Array.isArray(keys) ? keys : [keys]).map((key) => [key, state[key]])),
        set: async () => {},
      },
      session: {
        get: async () => ({ mediaSyncLast: {} }),
        set: async () => {},
      },
    },
    tabs: {
      query: async () => [],
      onUpdated: { addListener: (fn) => tabUpdatedListeners.push(fn) },
    },
    downloads: { onCreated: { addListener() {} } },
    contextMenus: { onClicked: { addListener() {} }, removeAll: (fn) => fn?.(), create() {} },
    notifications: { create() {} },
    cookies: {
      getAllCookieStores: async () => [{ id: "normal", tabIds: [11] }],
      getAll: async ({ url }) => {
        requestedCookieUrls.push(url);
        return currentBaiduCookies;
      },
    },
  };

  globalThis.fetch = async (url, options) => {
    assert.equal(url, "http://127.0.0.1:17433/v1/media/credentials/sync");
    syncPayloads.push(JSON.parse(options.body));
    return new Response("{}", { status: 200 });
  };

  try {
    await import("./background.js");

    // 1. Visiting www.baidu.com without BDUSS does not push incomplete cookies.
    for (const fn of tabUpdatedListeners) {
      fn(11, { status: "complete" }, { id: 11, incognito: false, url: "https://www.baidu.com/s?wd=test" });
    }
    await new Promise((resolve) => setTimeout(resolve, 60));
    assert.equal(syncPayloads.length, 0);

    // 2. Logging in on pan.baidu.com reads pan.baidu.com cookies and syncs immediately.
    currentBaiduCookies = [
      { name: "BDUSS", value: "bduss-v1" },
      { name: "STOKEN", value: "stoken-v1" },
    ];
    for (const fn of tabUpdatedListeners) {
      fn(11, { status: "complete" }, { id: 11, incognito: false, url: "https://pan.baidu.com/disk/main" });
    }
    await new Promise((resolve) => setTimeout(resolve, 60));
    assert.equal(syncPayloads.length, 1);
    assert.equal(syncPayloads[0].domain, "pan.baidu.com");
    assert.equal(syncPayloads[0].cookie, "BDUSS=bduss-v1; STOKEN=stoken-v1");
    assert.ok(requestedCookieUrls.every((u) => u === "https://pan.baidu.com/"));

    // 3. Re-logging in within 5 minutes updates cookie fingerprint and triggers immediate re-sync.
    currentBaiduCookies = [
      { name: "BDUSS", value: "bduss-v2" },
      { name: "STOKEN", value: "stoken-v2" },
    ];
    for (const fn of tabUpdatedListeners) {
      fn(11, { status: "complete" }, { id: 11, incognito: false, url: "https://yun.baidu.com/disk/main" });
    }
    await new Promise((resolve) => setTimeout(resolve, 60));
    assert.equal(syncPayloads.length, 2);
    assert.equal(syncPayloads[1].cookie, "BDUSS=bduss-v2; STOKEN=stoken-v2");

    // 4. Manual sync-cookies forwards force: true and userAgent.
    const manualResp = await new Promise((resolve) => {
      messageListeners[0](
        { type: "sync-cookies", domain: "pan.baidu.com", cookie: "BDUSS=manual", force: true, userAgent: "UA/1" },
        {},
        resolve,
      );
    });
    assert.equal(manualResp.ok, true);
    assert.equal(syncPayloads.length, 3);
    assert.equal(syncPayloads[2].force, true);
    assert.equal(syncPayloads[2].user_agent, "UA/1");
  } finally {
    globalThis.chrome = previousChrome;
    globalThis.fetch = previousFetch;
  }
});
