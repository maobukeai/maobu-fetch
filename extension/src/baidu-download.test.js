import test from "node:test";
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { interceptBrowserDownload, resetNotificationCooldownsForTest } from "./interceptor.js";
import { signature } from "./protocol.js";

globalThis.crypto ??= webcrypto;

const token = "synthetic-bridge-token";
const initial = {
  id: 101,
  url: "https://pan.baidu.com/s/synthetic?pwd=synthetic-secret",
  referrer: "https://pan.baidu.com/s/synthetic",
  filename: "sample.zip",
  totalBytes: 2_000_000,
  state: "in_progress",
};
const fresh = {
  ...initial,
  referrer: "",
  finalUrl: "https://d.pcs.baidu.com/file?token=synthetic-secret&pin=0000",
  mime: "application/octet-stream",
  paused: true,
};

function setup(context, item = fresh, settings = {}) {
  const calls = [];
  const local = { bridgeToken: token };
  const session = {};
  const sessionWrites = [];
  const store = (values, writes) => ({
    get: async (key) => key in values ? { [key]: structuredClone(values[key]) } : {},
    set: async (entries) => {
      writes?.push(structuredClone(entries));
      Object.assign(values, structuredClone(entries));
    },
  });
  let current = item;
  const downloads = {
    pause: async () => calls.push("pause"),
    search: async () => [current],
    cancel: async () => calls.push("cancel"),
    erase: async () => calls.push("erase"),
    resume: async () => calls.push("resume"),
  };
  const originalChrome = globalThis.chrome;
  const originalFetch = globalThis.fetch;
  context.after(() => {
    globalThis.chrome = originalChrome;
    globalThis.fetch = originalFetch;
  });
  globalThis.chrome = {
    runtime: { id: "test-extension" },
    storage: { local: store(local), session: store(session, sessionWrites) },
  };
  resetNotificationCooldownsForTest();
  const notifications = [];
  const requests = [];
  let verify = () => new Response(null, { status: 204 });
  globalThis.fetch = async (url, request) => {
    requests.push({ url, ...request });
    calls.push("verify");
    return verify();
  };
  const sent = [];
  const options = {
    downloads,
    settings: { intercept: true, ...settings },
    runtimeId: "test-extension",
    wait: async () => {},
    sendTask: async (...args) => { sent.push(args); calls.push("send"); },
    notify: (...args) => notifications.push(args),
  };
  return {
    calls, local, session, sessionWrites, requests, sent, options, notifications,
    setVerify: (handler) => { verify = handler; },
    setCurrent: (item) => { current = item; },
  };
}

test("Baidu downloads verify the signed browser context before enqueue and cancellation", async (context) => {
  const { calls, requests, sent, options, session, sessionWrites } = setup(context);
  options.tab = { id: 11, url: "https://pan.baidu.com/" };
  options.cookies = {
    getAllCookieStores: async () => [{ id: "regular", tabIds: [11] }],
    getAll: async ({ url, storeId }) => {
      assert.equal(url, fresh.finalUrl);
      assert.equal(storeId, "regular");
      return [
        { name: "pcs", value: "synthetic-cdn-cookie", domain: "d.pcs.baidu.com", secure: true },
        { name: "pan", value: "synthetic-other-cookie", domain: "pan.baidu.com", secure: true },
      ];
    },
  };
  // The refreshed item omitted the referrer, so the initial browser referrer is retained.
  options.tab.url = "https://example.com/active-tab";
  assert.equal(await interceptBrowserDownload(initial, options), true);
  assert.deepEqual(calls, ["pause", "verify", "send", "cancel", "erase"]);
  assert.deepEqual(session.pendingTakeover, {});
  assert.ok(!JSON.stringify(sessionWrites).includes("synthetic-secret"));
  const [request] = requests;
  assert.equal(request.url, "http://127.0.0.1:17433/v1/tasks/verify-baidu-download");
  assert.equal(request.method, "POST");
  assert.equal(request.headers.Origin, "chrome-extension://test-extension");
  assert.equal(request.headers["X-Luma-Signature"], await signature(token, request.headers["X-Luma-Timestamp"], request.body));
  const body = JSON.parse(request.body);
  assert.equal(body.total_bytes, fresh.totalBytes);
  assert.equal(body.url, fresh.finalUrl);
  assert.equal(body.headers.Cookie, "pcs=synthetic-cdn-cookie");
  assert.equal(body.headers.Referer, initial.referrer);
  assert.equal(body.headers["x-qr-token"], undefined);
  assert.equal(body.headers["x-pin"], undefined);
  assert.deepEqual(sent, [[fresh.finalUrl, fresh.filename, { headers: body.headers }]]);
});

test("Baidu verification rejection, old desktop, offline and timeout preserve browser download", async (context) => {
  for (const status of [403, 404, 200, "offline", "timeout"]) {
    await context.test(String(status), async (subtest) => {
      const mock = setup(subtest);
      mock.setVerify(() => {
        if (status === "offline") throw new TypeError("Failed to fetch synthetic-secret");
        if (status === "timeout") throw new DOMException("Request aborted synthetic-secret", "AbortError");
        return new Response("synthetic-secret", { status });
      });
      assert.equal(await interceptBrowserDownload(initial, mock.options), false);
      assert.deepEqual(mock.calls, status === "offline" || status === "timeout"
        ? ["pause", "verify", "verify", "resume"] : ["pause", "verify", "resume"]);
      assert.deepEqual(mock.session.pendingTakeover, {});
      assert.equal(mock.local.lastIgnored.url, "");
      assert.ok(!JSON.stringify([mock.local.lastIgnored, mock.notifications]).includes("synthetic-secret"));
    });
  }
});

test("Baidu leaves unknown targets, unknown sizes and unreplayable downloads with the browser", async (context) => {
  const scenarios = [
    ["no final URL", { finalUrl: "" }, {}, "baidu-unverified-target"],
    ["unknown target", { finalUrl: "https://example.com/file?token=synthetic-secret" }, { referrer: "https://yun.baidu.com/" }, "baidu-unverified-target"],
    ["lookalike target", { finalUrl: "https://baidupcs.com.example/file" }, {}, "baidu-unverified-target"],
    ["insecure target", { finalUrl: "http://d.pcs.baidu.com/file" }, {}, "baidu-unverified-target"],
    ["unknown size", { totalBytes: -1 }, {}, "baidu-size-unknown"],
    ["zero size", { totalBytes: 0 }, {}, "baidu-size-unknown"],
    ["unsafe size", { totalBytes: Number.MAX_SAFE_INTEGER + 1 }, {}, "baidu-size-unknown"],
    ["unsafe MIME", { mime: "Application/Problem+JSON; charset=utf-8" }, {}, "baidu-browser-only"],
    ["unsafe method", {}, { method: "PUT" }, "baidu-browser-only"],
  ];
  for (const [name, changes, start, reason] of scenarios) {
    await context.test(name, async (subtest) => {
      const mock = setup(subtest, { ...fresh, ...changes });
      assert.equal(await interceptBrowserDownload({ ...initial, ...start }, mock.options), false);
      assert.deepEqual(mock.calls, ["pause", "resume"]);
      assert.equal(mock.local.lastIgnored.reason, reason);
      assert.equal(mock.local.lastIgnored.url, "");
      assert.deepEqual(mock.session.pendingTakeover, {});
    });
  }
});

test("Baidu allows existing bypass and size rules to win before verification", async (context) => {
  for (const [reason, settings] of [["bypass", { bypassUntil: Date.now() + 60_000 }], ["size", { minSizeMb: 5 }]]) {
    await context.test(reason, async (subtest) => {
      const mock = setup(subtest, fresh, settings);
      assert.equal(await interceptBrowserDownload(initial, mock.options), false);
      assert.deepEqual(mock.calls, ["resume"]);
      assert.equal(mock.local.lastIgnored.reason, reason);
    });
  }
});

test("Baidu keeps browser ownership if the verified response URL has changed", async (context) => {
  const mock = setup(context);
  mock.setVerify(() => {
    mock.setCurrent({ ...fresh, finalUrl: "https://other.baidupcs.com/new" });
    return new Response(null, { status: 204 });
  });
  assert.equal(await interceptBrowserDownload(initial, mock.options), false);
  assert.deepEqual(mock.calls, ["pause", "verify", "resume"]);
});

test("unrelated downloads still enqueue without verification even with an unrelated Baidu tab active", async (context) => {
  const item = { ...fresh, referrer: "", url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip" };
  const mock = setup(context, item);
  mock.options.tab = { id: 11, url: "https://pan.baidu.com/" };
  assert.equal(await interceptBrowserDownload(item, mock.options), true);
  assert.deepEqual(mock.calls, ["pause", "send", "cancel", "erase"]);
});
