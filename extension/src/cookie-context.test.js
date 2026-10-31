import test from "node:test";
import assert from "node:assert/strict";
import { readTargetCookies } from "./cookie-context.js";
import { getDownloadAuthHeaders } from "./interceptor.js";
import { buildNetscapeCookiesText, sendWithCurrentPageAuth } from "./auth-download.js";

test("CookieStore.tabIds selects private identity and never guesses the normal store", async () => {
  const calls = [];
  const api = {
    getAllCookieStores: async () => [{ id: "normal", tabIds: [1] }, { id: "private", tabIds: [2] }],
    getAll: async (params) => { calls.push(params); return [{ name: "session", value: params.storeId }]; },
  };
  assert.equal((await readTargetCookies(api, "https://cdn.example/file", { tab: { id: 2, incognito: true } }))[0].value, "private");
  assert.deepEqual(await readTargetCookies(api, "https://cdn.example/file", { tab: { id: 99, incognito: true } }), []);
  assert.equal(calls.length, 1);
});

test("Download headers read only the final target; page/query credentials do not leak to CDN", async () => {
  const calls = [];
  const headers = await getDownloadAuthHeaders({
    url: "https://page.example/redirect?token=SOURCE", finalUrl: "https://cdn.example/file",
    referrer: "https://page.example/?pin=PAGE_PIN",
  }, { tab: { id: 1, url: "https://page.example/" }, cookies: {
    getAllCookieStores: async () => [{ id: "normal", tabIds: [1] }],
    getAll: async ({ url }) => { calls.push(url); return url.includes("page.example") ? [{ name: "secret", value: "SOURCE" }] : []; },
  } });
  assert.deepEqual(calls, ["https://cdn.example/file"]);
  assert.equal(headers.Cookie, undefined);
  assert.equal(headers["x-pin"], undefined);
  assert.equal(headers["x-qr-token"], undefined);
});

test("Cookies retain same-name path order and read only the actual frame partition", async () => {
  const calls = [];
  const api = {
    getAllCookieStores: async () => [{ id: "private", tabIds: [2] }],
    getPartitionKey: async (details) => { assert.deepEqual(details, { tabId: 2, frameId: 0 }); return { partitionKey: { topLevelSite: "https://page.example", hasCrossSiteAncestor: false } }; },
    getAll: async (params) => {
      calls.push(params);
      return params.partitionKey
        ? [{ name: "partition", value: "MATCH", domain: "cdn.example", path: "/", partitionKey: params.partitionKey }]
        : [{ name: "sid", value: "ROOT", path: "/" }, { name: "sid", value: "NESTED", path: "/files" }, { name: "wrong", value: "NO", domain: "other.example" }];
    },
  };
  const result = await readTargetCookies(api, "https://cdn.example/files/video.mp4", { tab: { id: 2 } });
  assert.deepEqual(result.map((cookie) => cookie.value), ["NESTED", "ROOT", "MATCH"]);
  assert.equal(calls.length, 2);
  assert.equal(calls[1].partitionKey.topLevelSite, "https://page.example");
});

test("One-shot download does not borrow page Cookie when the user enters another target", async () => {
  const calls = [];
  const result = await sendWithCurrentPageAuth({
    url: "https://cdn.example/file", pageUrl: "https://account.example/private", tab: { id: 1 },
    cookiesApi: { getAllCookieStores: async () => [{ id: "normal", tabIds: [1] }], getAll: async ({ url }) => { calls.push(url); return []; } },
    sendMessage: async () => { throw new Error("must not send without matching credentials"); },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(calls, ["https://cdn.example/file"]);
});

test("Netscape export preserves host/path scope and never turns CHIPS into unpartitioned credentials", () => {
  const text = buildNetscapeCookiesText([
    { name: "host", value: "HOST", domain: "www.youtube.com", hostOnly: true, secure: false, path: "/private" },
    { name: "domain", value: "DOMAIN", domain: ".youtube.com", hostOnly: false, secure: true },
    { name: "partition", value: "MUST_NOT_EXPORT", domain: ".youtube.com", partitionKey: { topLevelSite: "https://other.example" } },
  ], "https://www.youtube.com/private");
  assert.match(text, /www.youtube.com\tFALSE\t\/private\tFALSE/);
  assert.match(text, /\.youtube.com\tTRUE\t\/\tTRUE/);
  assert.equal(text.includes("MUST_NOT_EXPORT"), false);
});
