import test from "node:test";
import assert from "node:assert/strict";
import { matchMediaDomain, MEDIA_SYNC_DOMAINS } from "./domains.js";

test("PikPak 域名识别与基域归一化", () => {
  assert.equal(matchMediaDomain("mypikpak.com"), "mypikpak.com");
  assert.equal(matchMediaDomain("api-drive.mypikpak.com"), "mypikpak.com");
  assert.equal(matchMediaDomain("user.mypikpak.com"), "mypikpak.com");
  assert.equal(matchMediaDomain("mypikpak.net"), "mypikpak.com");
  assert.equal(matchMediaDomain("dl-a10b-1558.mypikpak.com"), "mypikpak.com");
});

test("MEDIA_SYNC_DOMAINS 包含 mypikpak 且无重复", () => {
  assert.ok(MEDIA_SYNC_DOMAINS.includes("mypikpak.com"));
  assert.ok(MEDIA_SYNC_DOMAINS.includes("mypikpak.net"));
  assert.equal(new Set(MEDIA_SYNC_DOMAINS).size, MEDIA_SYNC_DOMAINS.length);
});
