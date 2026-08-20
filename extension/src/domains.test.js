import test from "node:test";
import assert from "node:assert/strict";
import { matchMediaDomain, MEDIA_SYNC_DOMAINS } from "./domains.js";

test("matchMediaDomain: 精确域名与子域命中", () => {
  assert.equal(matchMediaDomain("bilibili.com"), "bilibili.com");
  assert.equal(matchMediaDomain("www.bilibili.com"), "bilibili.com");
  assert.equal(matchMediaDomain("m.douyin.com"), "douyin.com");
});

test("matchMediaDomain: 别名域归一化到基域", () => {
  assert.equal(matchMediaDomain("weibo.cn"), "weibo.com");
  assert.equal(matchMediaDomain("x.com"), "twitter.com");
  assert.equal(matchMediaDomain("mobile.x.com"), "twitter.com");
});

test("matchMediaDomain: 非媒体平台与相似域名不命中", () => {
  assert.equal(matchMediaDomain("example.com"), null);
  assert.equal(matchMediaDomain("notbilibili.com"), null);
  assert.equal(matchMediaDomain(""), null);
  assert.equal(matchMediaDomain(null), null);
  // 后缀相似但非子域（evilbilibili.com.x.com 之外的反例：bilibili.com.evil.com）
  assert.equal(matchMediaDomain("bilibili.com.evil.com"), null);
});

test("MEDIA_SYNC_DOMAINS: 覆盖已知平台且无重复", () => {
  assert.equal(new Set(MEDIA_SYNC_DOMAINS).size, MEDIA_SYNC_DOMAINS.length);
  for (const domain of [
    "douyin.com",
    "tiktok.com",
    "bilibili.com",
    "youtube.com",
    "twitter.com",
    "weibo.com",
    "123pan.com",
    "123pan.cn",
    "lanzoux.com",
    "lanzoui.com",
  ]) {
    assert.ok(MEDIA_SYNC_DOMAINS.includes(domain), `缺少 ${domain}`);
  }
  assert.equal(matchMediaDomain("1683912.share.123pan.cn"), "123pan.com");
  assert.equal(matchMediaDomain("www.lanzoui.com"), "lanzoux.com");
});

// ---- 用户自定义同步域名（P3）----

test("matchMediaDomain: 自定义域名参与匹配（精确与子域，大小写归一）", () => {
  assert.equal(matchMediaDomain("example.com", ["example.com"]), "example.com");
  assert.equal(matchMediaDomain("www.example.com", ["EXAMPLE.com"]), "example.com");
  assert.equal(matchMediaDomain("example.com", []), null);
  assert.equal(matchMediaDomain("other.com", ["example.com"]), null);
});

test("matchMediaDomain: 内置平台优先于自定义且互不干扰", () => {
  assert.equal(matchMediaDomain("bilibili.com", ["example.com"]), "bilibili.com");
  assert.equal(matchMediaDomain("sub.example.com", ["example.com", "a.com"]), "example.com");
});
