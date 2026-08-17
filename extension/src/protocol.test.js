import test from "node:test";
import assert from "node:assert/strict";
import { createHash, createHmac, webcrypto } from "node:crypto";
import { signature, compatFetch, signedFetch, focusDesktop, DEFAULT_TIMEOUT_MS } from "./protocol.js";

globalThis.crypto ??= webcrypto;

test("bridge signature matches the Rust HMAC protocol", async () => {
  const token = "0123456789abcdef";
  const timestamp = "1784419200000";
  const body = JSON.stringify({ url: "https://example.com/file.zip" });
  const key = createHash("sha256").update(token).digest();
  const expected = createHmac("sha256", key).update(`${timestamp}\n${body}`).digest("hex");
  assert.equal(await signature(token, timestamp, body), expected);
});

// ---- P0-3：请求超时 ----
// 桌面端进程僵死（TCP 通但不响应）时，请求必须按 timeoutMs 中止，
// 否则接管流程会把浏览器下载永久留在暂停状态。

function makeChromeMock(storage = {}, spies = {}) {
  return {
    storage: {
      local: {
        get: async (keys) => {
          const result = {};
          for (const key of Array.isArray(keys) ? keys : [keys]) {
            if (key in storage) result[key] = storage[key];
          }
          return result;
        },
        remove: async (keys) => { spies.removed ||= []; spies.removed.push(...(Array.isArray(keys) ? keys : [keys])); },
        set: async () => {},
      },
    },
    runtime: { id: "test-extension-id" },
  };
}

function withFetch(mock, fn) {
  const original = globalThis.fetch;
  globalThis.fetch = mock;
  return Promise.resolve(fn()).finally(() => { globalThis.fetch = original; });
}

test("timeout: compatFetch 按 timeoutMs 中止僵死连接", async () => {
  globalThis.chrome = makeChromeMock();
  globalThis.fetch = (_url, options = {}) => new Promise((_resolve, reject) => {
    options.signal?.addEventListener("abort", () => {
      const error = new Error("signal timed out");
      error.name = "AbortError";
      reject(error);
    });
  });
  try {
    await assert.rejects(
      compatFetch("/v1/health", { timeoutMs: 40 }),
      (error) => error?.name === "AbortError" || /timed?\s*out|abort/i.test(String(error?.message)),
    );
  } finally {
    delete globalThis.chrome;
  }
});

test("timeout: signedFetch 支持调用方覆盖超时且默认值在合理范围", async () => {
  assert.ok(DEFAULT_TIMEOUT_MS > 0 && DEFAULT_TIMEOUT_MS <= 10_000, "默认超时应在合理范围内");
  globalThis.chrome = makeChromeMock({ bridgeToken: "token" });
  globalThis.fetch = (_url, options = {}) => new Promise((_resolve, reject) => {
    options.signal?.addEventListener("abort", () => {
      const error = new Error("This operation was aborted");
      error.name = "AbortError";
      reject(error);
    });
  });
  try {
    await assert.rejects(
      signedFetch("/v1/tasks", { url: "https://example.com/a.zip" }, { timeoutMs: 40 }),
      (error) => /abort/i.test(String(error?.name || error?.message)),
    );
  } finally {
    delete globalThis.chrome;
  }
});

// ---- P1-6（roadmap F-13）：401 先 health 二次确认再清令牌 ----

test("401 handling: health 正常（桥接活着）时确认令牌失效并清除", async () => {
  const spies = {};
  globalThis.chrome = makeChromeMock({ bridgeToken: "stale-token" }, spies);
  const calls = [];
  try {
    await withFetch(async (url) => {
      calls.push(String(url));
      if (String(url).includes("/v1/health")) return { ok: true, status: 200, json: async () => ({}) };
      return { ok: false, status: 401, text: async () => "签名验证失败" };
    }, async () => signedFetch("/v1/tasks/recent", {}));
    assert.ok(spies.removed?.includes("bridgeToken"), "health 正常时必须清除失效令牌");
    assert.ok(calls.some((url) => url.includes("/v1/health")), "必须先做 health 二次确认");
  } finally {
    delete globalThis.chrome;
  }
});

test("401 handling: health 不可达时保留令牌（可能是临时异常/旧版本）", async () => {
  const spies = {};
  globalThis.chrome = makeChromeMock({ bridgeToken: "maybe-valid" }, spies);
  try {
    await withFetch(async (url) => {
      if (String(url).includes("/v1/health")) throw new TypeError("Failed to fetch");
      return { ok: false, status: 401, text: async () => "签名验证失败" };
    }, async () => signedFetch("/v1/tasks/recent", {}));
    assert.ok(!spies.removed?.includes("bridgeToken"), "health 不可达时不得清除令牌");
  } finally {
    delete globalThis.chrome;
  }
});

// ---- /v1/focus：通知点击唤起桌面端 ----

test("focusDesktop: 签名 POST /v1/focus，成功返回 true", async () => {
  globalThis.chrome = makeChromeMock({ bridgeToken: "token" });
  const calls = [];
  try {
    const ok = await withFetch(async (url, options = {}) => {
      calls.push({ url: String(url), method: options.method });
      return { ok: true, status: 200, json: async () => ({}) };
    }, () => focusDesktop());
    assert.equal(ok, true);
    assert.ok(calls.some((call) => call.url.includes("/v1/focus") && call.method === "POST"));
  } finally {
    delete globalThis.chrome;
  }
});

test("focusDesktop: 桌面端离线（未配对/连接失败）返回 false 不抛错", async () => {
  globalThis.chrome = makeChromeMock({});
  try {
    const ok = await withFetch(async () => { throw new TypeError("Failed to fetch"); }, () => focusDesktop());
    assert.equal(ok, false);
  } finally {
    delete globalThis.chrome;
  }
});
