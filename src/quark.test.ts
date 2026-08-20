/**
 * 夸克网盘解析单元测试（quark.ts）。
 *
 * 覆盖：URL 格式识别、提取码解析（query/文本）、分享 ID 提取。
 * 遵循项目零外部测试框架规范（AGENTS.md §8），通过 `npx tsx src/quark.test.ts` 运行。
 */

import { isQuarkUrl, parseQuarkUrl } from "./services/quark";

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(
      `${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(
        actual
      )}`
    );
  }
}

function assertTrue(value: unknown, message: string): void {
  if (!value) {
    throw new Error(`assertion failed: ${value}`);
  }
}

// 1. isQuarkUrl 测试
console.log("▶ 测试 isQuarkUrl...");
assertEqual(isQuarkUrl("https://pan.quark.cn/s/69ba75a686aa#/list/share"), true, "标准分享链接");
assertEqual(isQuarkUrl("http://quark.cn/s/abc123xyz"), true, "短域名分享链接");
assertEqual(isQuarkUrl("https://drive.quark.cn/s/test_id"), true, "drive 子域名");
assertEqual(isQuarkUrl("https://mypikpak.com/s/abc"), false, "非夸克链接");
assertEqual(isQuarkUrl("https://example.com/s/123"), false, "普通 URL");
assertEqual(isQuarkUrl(""), false, "空字符串");
console.log("✔ isQuarkUrl 测试通过");

// 2. parseQuarkUrl 测试
console.log("▶ 测试 parseQuarkUrl...");
const res1 = parseQuarkUrl("https://pan.quark.cn/s/69ba75a686aa#/list/share");
assertTrue(res1 !== null, "res1 非空");
if (res1) {
  assertEqual(res1.pwdId, "69ba75a686aa", "提取 pwdId");
  assertEqual(res1.passCode, undefined, "无提取码");
}

const res2 = parseQuarkUrl("https://pan.quark.cn/s/69ba75a686aa?pwd=abcd");
assertTrue(res2 !== null, "res2 非空");
if (res2) {
  assertEqual(res2.pwdId, "69ba75a686aa", "提取 pwdId");
  assertEqual(res2.passCode, "abcd", "提取 passCode");
}

const res3 = parseQuarkUrl("链接：https://pan.quark.cn/s/xyz987 提取码：1234 复制这段内容后打开夸克");
assertTrue(res3 !== null, "res3 非空");
if (res3) {
  assertEqual(res3.pwdId, "xyz987", "从文本中提取 pwdId");
  assertEqual(res3.passCode, "1234", "从文本中提取 passCode");
}
console.log("✔ parseQuarkUrl 测试通过");
