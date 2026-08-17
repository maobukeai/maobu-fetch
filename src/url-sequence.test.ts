/**
 * 批量序号 URL 展开单元测试（url-sequence.ts）。
 *
 * 覆盖：基础区间、零填充、步长、多组笛卡尔积、上限保护、
 * 语法错误、非 http(s) 行原样返回。
 *
 * 与 i18n.test.ts 相同的极简断言运行器（AGENTS.md §8 不引入测试框架），
 * 通过 `npx tsx src/url-sequence.test.ts` 执行，挂载在 `pnpm run check`。
 */

declare const process: { exitCode: number; argv: string[] };

import { expandSequenceUrls, isBtSourceLine, MAX_SEQUENCE_EXPANSION } from "./url-sequence.js";

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

type TestCase = { name: string; fn: () => void };
const tests: TestCase[] = [];
const test = (name: string, fn: () => void) => { tests.push({ name, fn }); };

// ===== 展开行为 =====

test("无序号组的 URL 原样返回", () => {
  const result = expandSequenceUrls("https://example.com/file.zip");
  assertEqual(result.urls.length, 1, "single url");
  assertEqual(result.urls[0], "https://example.com/file.zip", "unchanged");
  assertEqual(result.error, undefined, "no error");
});

test("基础区间 [1-3] 展开为 3 个 URL", () => {
  const result = expandSequenceUrls("https://example.com/file[1-3].zip");
  assertEqual(result.urls.join("|"), "https://example.com/file1.zip|https://example.com/file2.zip|https://example.com/file3.zip", "expanded");
});

test("零填充按操作数最大宽度推断", () => {
  const result = expandSequenceUrls("https://example.com/pic[001-003].jpg");
  assertEqual(result.urls.join("|"), "https://example.com/pic001.jpg|https://example.com/pic002.jpg|https://example.com/pic003.jpg", "zero padded");
});

test("宽度不一致时取较大宽度", () => {
  const result = expandSequenceUrls("https://example.com/f[07-12].bin");
  assertEqual(result.urls[0], "https://example.com/f07.bin", "first padded");
  assertEqual(result.urls[5], "https://example.com/f12.bin", "last padded");
  assertEqual(result.urls.length, 6, "count");
});

test("步长 [0-10:5] 按步进取值（宽度按操作数最大值 2 补零）", () => {
  const result = expandSequenceUrls("https://example.com/s[0-10:5].zip");
  assertEqual(result.urls.join("|"), "https://example.com/s00.zip|https://example.com/s05.zip|https://example.com/s10.zip", "stepped");
});

test("步长大于区间时仅取起点", () => {
  const result = expandSequenceUrls("https://example.com/s[1-3:10].zip");
  assertEqual(result.urls.join("|"), "https://example.com/s1.zip", "single value");
});

test("多个序号组做笛卡尔积", () => {
  const result = expandSequenceUrls("https://example.com/a[1-2]-b[3-4].zip");
  assertEqual(
    result.urls.join("|"),
    "https://example.com/a1-b3.zip|https://example.com/a1-b4.zip|https://example.com/a2-b3.zip|https://example.com/a2-b4.zip",
    "cartesian"
  );
});

test("展开总数超过上限返回错误", () => {
  const result = expandSequenceUrls(`https://example.com/f[1-${MAX_SEQUENCE_EXPANSION + 1}].zip`);
  assertEqual(result.urls.length, 0, "no urls on error");
  assertTrue(result.error !== undefined && result.error.includes("超过上限"), "error mentions limit");
});

test("超大区间在构建数组前即被拒绝（不冻结 UI）", () => {
  // 回归：groupValues 曾先物化完整数组再由调用方兜底，
  // [1-999999999] 会循环上亿次。现在必须立即返回错误。
  const result = expandSequenceUrls("https://example.com/f[1-999999999].zip");
  assertEqual(result.urls.length, 0, "no urls on error");
  assertTrue(result.error !== undefined && result.error.includes("超过上限"), "error mentions limit");
});

test("恰好等于上限的单组仍可展开", () => {
  const result = expandSequenceUrls(`https://example.com/f[1-${MAX_SEQUENCE_EXPANSION}].zip`);
  assertEqual(result.error, undefined, "at limit is allowed");
  assertEqual(result.urls.length, MAX_SEQUENCE_EXPANSION, "exact count");
});

test("两个 200×… 组合的超限在第二组即报错", () => {
  const result = expandSequenceUrls("https://example.com/a[1-150]-b[1-150].zip");
  assertTrue(result.error !== undefined && result.error.includes("超过上限"), "cartesian limit");
});

// ===== 语法错误 =====

test("起点大于终点报错", () => {
  const result = expandSequenceUrls("https://example.com/f[5-3].zip");
  assertTrue(result.error !== undefined && result.error.includes("起点不能大于终点"), "start>end error");
});

test("步长为 0 报错", () => {
  const result = expandSequenceUrls("https://example.com/f[1-9:0].zip");
  assertTrue(result.error !== undefined && result.error.includes("步长"), "zero step error");
});

// ===== 非 http(s) 行 =====

test("magnet: 行原样返回不展开", () => {
  const magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=a[1-2]";
  const result = expandSequenceUrls(magnet);
  assertEqual(result.urls.length, 1, "single");
  assertEqual(result.urls[0], magnet, "unchanged");
});

test("本地 .torrent 路径原样返回", () => {
  const path = "D:/downloads/ubuntu.torrent";
  const result = expandSequenceUrls(path);
  assertEqual(result.urls[0], path, "unchanged");
});

test("空行返回空列表", () => {
  const result = expandSequenceUrls("   ");
  assertEqual(result.urls.length, 0, "empty");
});

// ===== BT 来源识别 =====

test("isBtSourceLine 识别 magnet 与 .torrent，排除 http", () => {
  assertTrue(isBtSourceLine("magnet:?xt=urn:btih:abc"), "magnet");
  assertTrue(isBtSourceLine("MAGNET:?xt=urn:btih:abc"), "magnet case-insensitive");
  assertTrue(isBtSourceLine("D:\\downloads\\a.torrent"), "windows torrent path");
  assertTrue(isBtSourceLine("/home/user/a.torrent"), "unix torrent path");
  assertTrue(!isBtSourceLine("https://example.com/a.torrent"), "remote torrent url is http");
  assertTrue(!isBtSourceLine("https://example.com/file.zip"), "plain http");
  assertTrue(!isBtSourceLine("随便一行文本"), "plain text");
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

// 当作为脚本直接执行时运行测试；被 import 时不自动运行。
// import.meta.url 对非 ASCII 路径（如本项目的"下载器"目录）是百分号编码，
// 必须 decodeURI 后再与 argv 比较，否则入口检测在中文路径下静默失效。
if (typeof process !== "undefined" && process.argv[1] && decodeURI(import.meta.url).endsWith(process.argv[1].replace(/\\/g, "/"))) {
  runAllTests();
}

export { runAllTests };
