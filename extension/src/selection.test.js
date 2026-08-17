import test from "node:test";
import assert from "node:assert/strict";
import { parseSelectionHash } from "./selection.js";

const encode = (links) => `#${encodeURIComponent(JSON.stringify(links))}`;

test("parseSelectionHash: 解析合法载荷并保序", () => {
  const links = [
    "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=test",
    "https://example.com/a.zip",
  ];
  assert.deepEqual(parseSelectionHash(encode(links)), links);
});

test("parseSelectionHash: 去重并过滤非法条目", () => {
  const hash = encode([
    "https://example.com/a.zip",
    "https://example.com/a.zip", // 重复
    "javascript:alert(1)",       // 非 http(s)/magnet
    "magnet:?no-hash",           // 无 infohash 的磁力
    "",
  ]);
  assert.deepEqual(parseSelectionHash(hash), ["https://example.com/a.zip"]);
});

test("parseSelectionHash: 空或损坏载荷返回空数组", () => {
  assert.deepEqual(parseSelectionHash(""), []);
  assert.deepEqual(parseSelectionHash("#"), []);
  assert.deepEqual(parseSelectionHash("#%E4%B8%8D%E6%98%AFJSON"), []);
  assert.deepEqual(parseSelectionHash(encode("not-an-array")), []);
});
