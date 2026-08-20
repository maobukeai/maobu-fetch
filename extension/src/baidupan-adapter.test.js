import assert from "node:assert/strict";
import test from "node:test";
import { matchMediaDomain, MEDIA_SYNC_DOMAINS } from "./domains.js";

test("BaiduPan 域名识别与基域归一化", () => {
  assert.equal(matchMediaDomain("pan.baidu.com"), "pan.baidu.com");
  assert.equal(matchMediaDomain("yun.baidu.com"), "pan.baidu.com");
  assert.equal(matchMediaDomain("baidu.com"), "pan.baidu.com");
  assert.equal(matchMediaDomain("sub.pan.baidu.com"), "pan.baidu.com");
});

test("MEDIA_SYNC_DOMAINS 包含 baidu 且无重复", () => {
  assert.ok(MEDIA_SYNC_DOMAINS.includes("pan.baidu.com"));
  assert.ok(MEDIA_SYNC_DOMAINS.includes("yun.baidu.com"));
  assert.ok(MEDIA_SYNC_DOMAINS.includes("baidu.com"));
  const set = new Set(MEDIA_SYNC_DOMAINS);
  assert.equal(set.size, MEDIA_SYNC_DOMAINS.length, "不应有重复项");
});
