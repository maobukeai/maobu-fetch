/**
 * 百度网盘解析单元测试（baidupan.ts）。
 *
 * 覆盖：URL 格式识别、提取码解析（query/文本）、surl 提取。
 * 遵循项目零外部测试框架规范（AGENTS.md §8），通过 `npx tsx src/baidupan.test.ts` 运行。
 */

import { isBaiduUrl, parseBaiduUrl } from "./services/baidupan";

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

// 1. isBaiduUrl 测试
console.log("▶ 测试 isBaiduUrl...");
assertEqual(isBaiduUrl("https://pan.baidu.com/s/1abcdefg"), true, "标准 1 开头分享链接");
assertEqual(isBaiduUrl("https://pan.baidu.com/s/xyz123"), true, "非 1 开头分享链接");
assertEqual(isBaiduUrl("http://yun.baidu.com/s/1abcdefg"), true, "yun.baidu.com 链接");
assertEqual(isBaiduUrl("https://pan.baidu.com/share/init?surl=abcxyz"), true, "init?surl 链接");
assertEqual(isBaiduUrl("https://pan.quark.cn/s/69ba75a686aa"), false, "夸克链接不误判");
assertEqual(isBaiduUrl("https://mypikpak.com/s/abc"), false, "PikPak 链接不误判");
assertEqual(isBaiduUrl(""), false, "空字符串");
console.log("✔ isBaiduUrl 测试通过");

// 2. parseBaiduUrl 测试
console.log("▶ 测试 parseBaiduUrl...");
const res1 = parseBaiduUrl("https://pan.baidu.com/s/1abcdefg");
assertTrue(res1 !== null, "res1 非空");
if (res1) {
  assertEqual(res1.surl, "abcdefg", "提取 surl");
  assertEqual(res1.passCode, undefined, "无提取码");
}

const res2 = parseBaiduUrl("https://pan.baidu.com/s/1abcdefg?pwd=1234");
assertTrue(res2 !== null, "res2 非空");
if (res2) {
  assertEqual(res2.surl, "abcdefg", "提取 surl");
  assertEqual(res2.passCode, "1234", "提取 passCode");
}

const res3 = parseBaiduUrl("链接: https://pan.baidu.com/s/1xyz789 提取码: abcd 复制这段内容后打开百度网盘手机App");
assertTrue(res3 !== null, "res3 非空");
if (res3) {
  assertEqual(res3.surl, "xyz789", "从文本中提取 surl");
  assertEqual(res3.passCode, "abcd", "从文本中提取 passCode");
}
console.log("✔ parseBaiduUrl 测试通过");
