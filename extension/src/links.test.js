import test from "node:test";
import assert from "node:assert/strict";
import { isMagnetUrl, isDownloadableMagnet, extractLinksFromText } from "./links.js";

test("isMagnetUrl：大小写与空白容忍，非磁力返回 false", () => {
  assert.equal(isMagnetUrl("magnet:?xt=urn:btih:abc"), true);
  assert.equal(isMagnetUrl("  MAGNET:?xt=urn:btih:abc "), true);
  assert.equal(isMagnetUrl("https://example.com/file.zip"), false);
  assert.equal(isMagnetUrl(""), false);
  assert.equal(isMagnetUrl(null), false);
});

test("isDownloadableMagnet：必须带 xt=urn:btih 才算可下载", () => {
  assert.equal(isDownloadableMagnet("magnet:?xt=urn:btih:0123456789abcdef"), true);
  assert.equal(isDownloadableMagnet("magnet:?xt=urn:btih:ABCDEF0123456789"), true);
  // 只有前缀、无 infohash → 不可下载（提前交给浏览器，避免桌面端报错）
  assert.equal(isDownloadableMagnet("magnet:?dn=only-name"), false);
  assert.equal(isDownloadableMagnet("magnet:"), false);
  assert.equal(isDownloadableMagnet("https://example.com/a.torrent"), false);
});

test("extractLinksFromText：磁力与 http(s) 各自去重保序", () => {
  const text = [
    "分享两个链接：",
    "magnet:?xt=urn:btih:aaa&dn=1",
    "https://example.com/a.zip",
    "magnet:?xt=urn:btih:aaa&dn=1", // 重复
    "magnet:?dn=no-hash",            // 无 infohash，丢弃
    "https://example.com/b.iso",
  ].join(" ");
  const { magnets, urls } = extractLinksFromText(text, 10);
  assert.deepEqual(magnets, ["magnet:?xt=urn:btih:aaa&dn=1"]);
  assert.deepEqual(urls, ["https://example.com/a.zip", "https://example.com/b.iso"]);
});

test("extractLinksFromText：剥离句尾标点并受 max 限制", () => {
  const text = "看这个 https://example.com/x.exe，还有 magnet:?xt=urn:btih:bbb。";
  const { magnets, urls } = extractLinksFromText(text, 10);
  assert.deepEqual(magnets, ["magnet:?xt=urn:btih:bbb"]);
  assert.deepEqual(urls, ["https://example.com/x.exe"]);
  const many = Array.from({ length: 8 }, (_, i) => `https://example.com/${i}.zip`).join(" ");
  assert.equal(extractLinksFromText(many, 3).urls.length, 3);
});

test("extractLinksFromText：引号/尖括号内的链接不越界", () => {
  const text = '<a href="https://example.com/in-tag.zip">x</a> magnet:?xt=urn:btih:ccc<finish>';
  const { magnets, urls } = extractLinksFromText(text, 10);
  assert.deepEqual(magnets, ["magnet:?xt=urn:btih:ccc"]);
  assert.deepEqual(urls, ["https://example.com/in-tag.zip"]);
});
