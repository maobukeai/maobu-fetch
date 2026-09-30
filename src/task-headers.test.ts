/**
 * 认证头辅助纯函数单元测试（task-headers.ts）。
 *
 * 覆盖：临时登录态判定（含 Authorization、大小写与空白变体）、
 * 模板保存前的认证头剔除、输入对象不被修改。
 *
 * 与项目其他前端测试相同的极简断言运行器（AGENTS.md §8 不引入测试框架），
 * 通过 `npx tsx src/task-headers.test.ts` 执行，挂载在 `pnpm run check`。
 */

declare const process: { exitCode: number; argv: string[] };

import {
  hasTemporaryAuthHeaders,
  isCredentialHeaderName,
  stripCredentialHeaders,
} from "./task-headers.js";

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertTrue(value: unknown, message: string): void {
  if (!value) {
    throw new Error(`${message}: expected truthy, got ${JSON.stringify(value)}`);
  }
}

function assertFalse(value: unknown, message: string): void {
  if (value) {
    throw new Error(`${message}: expected falsy, got ${JSON.stringify(value)}`);
  }
}

type TestCase = { name: string; fn: () => void };
const tests: TestCase[] = [];
const test = (name: string, fn: () => void) => { tests.push({ name, fn }); };

// ===== hasTemporaryAuthHeaders =====

test("识别 Cookie/Referer/User-Agent（原有行为保持）", () => {
  assertTrue(hasTemporaryAuthHeaders({ Cookie: "sid=1" }), "Cookie");
  assertTrue(hasTemporaryAuthHeaders({ cookie: "sid=1" }), "cookie");
  assertTrue(hasTemporaryAuthHeaders({ Referer: "https://x/" }), "Referer");
  assertTrue(hasTemporaryAuthHeaders({ referrer: "https://x/" }), "referrer");
  assertTrue(hasTemporaryAuthHeaders({ "User-Agent": "MaobuFetch" }), "User-Agent");
});

test("识别 Authorization（本次修复：此前被遗漏）", () => {
  assertTrue(hasTemporaryAuthHeaders({ Authorization: "Bearer x" }), "Authorization");
  assertTrue(hasTemporaryAuthHeaders({ authorization: "Bearer x" }), "authorization");
  assertTrue(hasTemporaryAuthHeaders({ AUTHORIZATION: "Bearer x" }), "AUTHORIZATION");
});

test("名称两侧空白不影响判定", () => {
  assertTrue(hasTemporaryAuthHeaders({ " Cookie ": "sid=1" }), "padded Cookie");
  assertTrue(hasTemporaryAuthHeaders({ " user-agent ": "x" }), "padded User-Agent");
});

test("普通请求头不触发临时登录态判定", () => {
  assertFalse(hasTemporaryAuthHeaders({ "X-Title": "a", Accept: "*/*" }), "plain headers");
  assertFalse(hasTemporaryAuthHeaders({}), "empty headers");
  assertFalse(hasTemporaryAuthHeaders(undefined), "undefined");
  assertFalse(hasTemporaryAuthHeaders(null), "null");
});

// ===== isCredentialHeaderName =====

test("认证凭据头判定大小写不敏感", () => {
  assertTrue(isCredentialHeaderName("Cookie"), "Cookie");
  assertTrue(isCredentialHeaderName("authorization"), "authorization");
  assertTrue(isCredentialHeaderName("Proxy-Authorization"), "Proxy-Authorization");
  assertFalse(isCredentialHeaderName("Referer"), "Referer is not a credential header");
  assertFalse(isCredentialHeaderName("X-Custom"), "X-Custom");
});

// ===== stripCredentialHeaders =====

test("剔除 Cookie 与 Authorization，保留 Referer/User-Agent", () => {
  const stripped = stripCredentialHeaders({
    Cookie: "sid=1",
    Authorization: "Bearer x",
    Referer: "https://x/",
    "User-Agent": "MaobuFetch",
  });
  assertEqual(JSON.stringify(stripped), JSON.stringify({ Referer: "https://x/", "User-Agent": "MaobuFetch" }), "credential headers stripped");
});

test("大小写变体同样被剔除", () => {
  const stripped = stripCredentialHeaders({ COOKIE: "sid=1", "user-agent": "x" });
  assertEqual(JSON.stringify(stripped), JSON.stringify({ "user-agent": "x" }), "COOKIE removed");
  const proxy = stripCredentialHeaders({ "PROXY-AUTHORIZATION": "Basic x", Accept: "*/*" });
  assertEqual(JSON.stringify(proxy), JSON.stringify({ Accept: "*/*" }), "proxy-authorization removed");
});

test("全部为认证头时返回 null", () => {
  assertEqual(stripCredentialHeaders({ Cookie: "sid=1" }), null, "only cookie");
  assertEqual(stripCredentialHeaders({}), null, "empty object");
  assertEqual(stripCredentialHeaders(null), null, "null input");
  assertEqual(stripCredentialHeaders(undefined), null, "undefined input");
});

test("不修改传入的请求头对象", () => {
  const original = { Cookie: "sid=1", Referer: "https://x/" };
  stripCredentialHeaders(original);
  assertEqual(JSON.stringify(original), JSON.stringify({ Cookie: "sid=1", Referer: "https://x/" }), "input not mutated");
});

function runAllTests(): void {
  let passed = 0;
  let failed = 0;
  const failures: string[] = [];
  for (const testCase of tests) {
    try {
      testCase.fn();
      passed += 1;
    } catch (error) {
      failed += 1;
      const message = error instanceof Error ? error.message : String(error);
      failures.push(`  ✗ ${testCase.name}: ${message}`);
    }
  }
  if (failed > 0) {
    console.error(`\nFailed ${failed} / ${tests.length} tests:`);
    for (const failure of failures) console.error(failure);
    process.exitCode = 1;
  } else {
    console.log(`\nPassed ${passed} / ${tests.length} tests.`);
  }
}

// 非 ASCII 路径下 import.meta.url 是百分号编码，必须 decodeURI 后再比较（见 url-sequence.test.ts）。
if (typeof process !== "undefined" && process.argv[1] && decodeURI(import.meta.url).endsWith(process.argv[1].replace(/\\/g, "/"))) {
  runAllTests();
}

export { runAllTests };
