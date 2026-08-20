import test from "node:test";
import assert from "node:assert/strict";
import { matchMediaDomain, MEDIA_SYNC_DOMAINS } from "./domains.js";

test("Quark 域名识别与基域归一化", () => {
  assert.equal(matchMediaDomain("pan.quark.cn"), "pan.quark.cn");
  assert.equal(matchMediaDomain("drive.quark.cn"), "pan.quark.cn");
  assert.equal(matchMediaDomain("quark.cn"), "pan.quark.cn");
  assert.equal(matchMediaDomain("sub.pan.quark.cn"), "pan.quark.cn");
});

test("MEDIA_SYNC_DOMAINS 包含 quark 且无重复", () => {
  assert.ok(MEDIA_SYNC_DOMAINS.includes("pan.quark.cn"));
  assert.ok(MEDIA_SYNC_DOMAINS.includes("drive.quark.cn"));
  assert.ok(MEDIA_SYNC_DOMAINS.includes("quark.cn"));
  assert.equal(new Set(MEDIA_SYNC_DOMAINS).size, MEDIA_SYNC_DOMAINS.length);
});
