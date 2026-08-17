/**
 * 拖放新建任务纯函数单元测试（drag-drop.ts）。
 *
 * 与项目其他前端测试相同的极简断言运行器（AGENTS.md §8 不引入测试框架），
 * 通过 `npx tsx src/drag-drop.test.ts` 执行，挂载在 `pnpm run check`。
 */

declare const process: { exitCode: number; argv: string[] };

import { arrayBufferToBase64, classifyDroppedFiles, extractDroppedUrls, MAX_DROPPED_URLS } from "./drag-drop.js";

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

// ===== URL 提取 =====

test("从多行文本提取 http 与 magnet 链接并去重", () => {
  const urls = extractDroppedUrls(
    "https://example.com/a.zip\nmagnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567\nhttps://example.com/a.zip\n随便的文字"
  );
  assertEqual(urls.length, 2, "deduped urls");
  assertEqual(urls[0], "https://example.com/a.zip", "http url");
  assertEqual(urls[1], "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567", "magnet url");
});

test("uri-list 中的注释行被忽略", () => {
  const urls = extractDroppedUrls("#\r\nhttps://example.com/b.zip\r\n");
  assertEqual(urls.length, 1, "comment ignored");
  assertEqual(urls[0], "https://example.com/b.zip", "url kept");
});

test("空白与非 URL 文本返回空数组", () => {
  assertEqual(extractDroppedUrls("").length, 0, "empty");
  assertEqual(extractDroppedUrls("hello world").length, 0, "plain text");
});

test("超过上限时截断到 MAX_DROPPED_URLS", () => {
  const many = Array.from({ length: MAX_DROPPED_URLS + 50 }, (_, i) => `https://example.com/f${i}.zip`).join("\n");
  assertEqual(extractDroppedUrls(many).length, MAX_DROPPED_URLS, "capped");
});

// ===== 文件分类 =====

test("按扩展名分类种子与不支持文件（大小写不敏感）", () => {
  const files = [
    { name: "ubuntu.torrent" },
    { name: "Movie.Torrent" },
    { name: "photo.jpg" },
    { name: "notes.txt" },
  ];
  const { torrents, rejected } = classifyDroppedFiles(files);
  assertEqual(torrents.length, 2, "torrent count");
  assertEqual(rejected.length, 2, "rejected count");
  assertEqual(torrents[0].name, "ubuntu.torrent", "first torrent");
  assertEqual(rejected[0].name, "photo.jpg", "first rejected");
});

// ===== base64 =====

test("ArrayBuffer 转 base64 与已知值一致", () => {
  const encoder = new TextEncoder();
  assertEqual(arrayBufferToBase64(encoder.encode("hello").buffer), "aGVsbG8=", "small buffer");
  // 大于 0x8000 分块的输入也能完整转换。
  const big = new Uint8Array(0x8000 + 100);
  big.fill(65);
  const base64 = arrayBufferToBase64(big.buffer);
  assertTrue(base64.length > 0x8000, "large buffer encoded");
  assertEqual(base64.length % 4, 0, "base64 length multiple of 4");
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
