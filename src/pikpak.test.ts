/**
 * PikPak 网盘解析单元测试（pikpak.ts）。
 *
 * 覆盖：自包含 MD5 标准用例（RFC 1321）、URL 格式识别、提取码解析（query/文本）、分享文件结构处理。
 * 遵循项目零外部测试框架规范（AGENTS.md §8），通过 `npx tsx src/pikpak.test.ts` 运行。
 */

import {
  md5,
  isPikPakShareUrl,
  parsePikPakShareUrl,
} from "./services/pikpak";

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
    throw new Error(
      `${message}: expected truthy, got ${JSON.stringify(value)}`
    );
  }
}

type TestCase = { name: string; fn: () => void };
const tests: TestCase[] = [];
const test = (name: string, fn: () => void) => {
  tests.push({ name, fn });
};

// 1. MD5 算法标准测试（RFC 1321 官方用例）
test("MD5 空字符串测试", () => {
  assertEqual(md5(""), "d41d8cd98f00b204e9800998ecf8427e", "MD5 空串");
});

test("MD5 简短字符与单词", () => {
  assertEqual(md5("a"), "0cc175b9c0f1b6a831c399e269772661", "MD5 'a'");
  assertEqual(md5("abc"), "900150983cd24fb0d6963f7d28e17f72", "MD5 'abc'");
  assertEqual(
    md5("message digest"),
    "f96b697d7cb7938d525a2f31aaf161d0",
    "MD5 'message digest'"
  );
  assertEqual(
    md5("abcdefghijklmnopqrstuvwxyz"),
    "c3fcd3d76192e4007dfb496cca67e13b",
    "MD5 alphabet"
  );
});

// 2. isPikPakShareUrl 识别测试
test("isPikPakShareUrl 识别合法与非法链接", () => {
  assertTrue(
    isPikPakShareUrl("https://mypikpak.com/s/VN_a123bc"),
    "标准 mypikpak.com/s/ 链接应识别"
  );
  assertTrue(
    isPikPakShareUrl("https://www.mypikpak.com/s/VN_a123bc/folder_id_456"),
    "带子目录的 mypikpak.com 链接应识别"
  );
  assertTrue(
    isPikPakShareUrl("http://mypikpak.net/s/ABCDEF123"),
    "mypikpak.net 备用域名应识别"
  );
  assertEqual(
    isPikPakShareUrl("https://api-drive.mypikpak.com/drive/v1/files"),
    false,
    "API URL 不应识别为分享链接"
  );
  assertEqual(
    isPikPakShareUrl("https://example.com/s/123"),
    false,
    "非 PikPak 链接不应识别"
  );
  assertEqual(isPikPakShareUrl(""), false, "空串不应通过");
});

// 3. parsePikPakShareUrl 解析提取测试
test("parsePikPakShareUrl 提取标准分享", () => {
  const single = parsePikPakShareUrl("https://mypikpak.com/s/VN_a123bc");
  assertTrue(single, "应成功解析");
  if (single) {
    assertEqual(single.shareId, "VN_a123bc", "shareId 匹配");
    assertEqual(single.parentId, undefined, "无 parentId");
    assertEqual(single.passCode, undefined, "无 passCode");
  }
});

test("parsePikPakShareUrl 提取子目录分享", () => {
  const withParent = parsePikPakShareUrl(
    "https://mypikpak.com/s/VN_a123bc/sub_folder_999"
  );
  assertTrue(withParent, "应成功解析子目录");
  if (withParent) {
    assertEqual(withParent.shareId, "VN_a123bc", "shareId 匹配");
    assertEqual(withParent.parentId, "sub_folder_999", "parentId 匹配");
  }
});

test("parsePikPakShareUrl 提取 URL 参数密码", () => {
  const withQueryPwd = parsePikPakShareUrl(
    "https://mypikpak.com/s/VN_a123bc?pwd=8888"
  );
  assertTrue(withQueryPwd, "应成功解析 query 密码");
  if (withQueryPwd) {
    assertEqual(withQueryPwd.shareId, "VN_a123bc", "shareId 匹配");
    assertEqual(withQueryPwd.passCode, "8888", "passCode 提取成功");
  }
});

test("parsePikPakShareUrl 提取文本中附带的提取码", () => {
  const withTextPwd = parsePikPakShareUrl(
    "快来下载：https://mypikpak.com/s/VN_a123bc 提取码: 6688 欢迎保存"
  );
  assertTrue(withTextPwd, "应成功解析文本提取码");
  if (withTextPwd) {
    assertEqual(withTextPwd.shareId, "VN_a123bc", "shareId 匹配");
    assertEqual(withTextPwd.passCode, "6688", "提取码解析成功");
  }
});

// 执行所有测试用例
let passed = 0;
for (const t of tests) {
  t.fn();
  passed += 1;
}

console.log(`\nPassed ${passed} / ${tests.length} tests.\n`);
