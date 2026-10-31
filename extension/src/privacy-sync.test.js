import test from "node:test";
import assert from "node:assert/strict";

test("Automatic credential sync ignores private tabs while preserving normal user-configured sites", async () => {
  const previousChrome = globalThis.chrome;
  const previousFetch = globalThis.fetch;
  const callbacks = [];
  const requests = [];
  let cookieReads = 0;
  const state = { bridgeToken: "synthetic-sync-token", customMediaDomains: ["account.example"] };
  globalThis.chrome = {
    runtime: { id: "synthetic", onMessage: { addListener() {} }, onInstalled: { addListener() {} } },
    storage: {
      local: { get: async (keys) => Object.fromEntries((Array.isArray(keys) ? keys : [keys]).map((key) => [key, state[key]])), set: async () => {} },
      session: { get: async () => ({}), set: async () => {} },
    },
    tabs: { query: async () => [], onUpdated: { addListener: (fn) => callbacks.push(fn) } },
    downloads: { onCreated: { addListener() {} } },
    contextMenus: { onClicked: { addListener() {} }, removeAll: (fn) => fn?.(), create() {} },
    notifications: { create() {} },
    cookies: {
      getAllCookieStores: async () => [{ id: "normal", tabIds: [5] }, { id: "private", tabIds: [6] }],
      getAll: async ({ storeId }) => { cookieReads += 1; return [{ name: "session", value: storeId }]; },
    },
  };
  globalThis.fetch = async (url, options) => {
    assert.equal(url, "http://127.0.0.1:17433/v1/media/credentials/sync");
    requests.push(JSON.parse(options.body));
    return new Response("{}", { status: 200 });
  };
  try {
    await import("./background.js");
    for (const callback of callbacks) callback(6, { status: "complete" }, { id: 6, incognito: true, url: "https://account.example/private" });
    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.equal(cookieReads, 0);
    assert.equal(requests.length, 0);
    for (const callback of callbacks) callback(5, { status: "complete" }, { id: 5, incognito: false, url: "https://account.example/private" });
    await new Promise((resolve) => setTimeout(resolve, 20));
    assert.equal(requests.length, 1);
    assert.equal(requests[0].cookie, "session=normal");
  } finally { globalThis.chrome = previousChrome; globalThis.fetch = previousFetch; }
});
