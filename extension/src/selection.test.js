import test from "node:test";
import assert from "node:assert/strict";
import { parseSelectionHash, categorizeLink } from "./selection.js";

const encode = (links) => `#${encodeURIComponent(JSON.stringify(links))}`;

test("parseSelectionHash: 解析合法载荷并保序", () => {
  const links = [
    "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=test",
    "https://example.com/a.zip",
  ];
  assert.deepEqual(parseSelectionHash(encode(links)), links);
});

test("parseSelectionHash: 支持对象结构 [{ url, title }] 并提取 url 去重", () => {
  const items = [
    { url: "https://example.com/movie.mp4", title: "示例视频" },
    { url: "https://example.com/song.flac", title: "无损音乐" },
    { url: "https://example.com/movie.mp4", title: "重复视频" },
  ];
  assert.deepEqual(parseSelectionHash(encode(items)), [
    "https://example.com/movie.mp4",
    "https://example.com/song.flac",
  ]);
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

test("categorizeLink: 准确识别各类媒体与资源", () => {
  assert.equal(categorizeLink("https://example.com/video.mp4").category, "video");
  assert.equal(categorizeLink("https://example.com/live/playlist.m3u8?token=123").category, "video");
  assert.equal(categorizeLink("https://example.com/audio.flac").category, "audio");
  assert.equal(categorizeLink("https://example.com/data.tar.gz").category, "archive");
  assert.equal(categorizeLink("https://example.com/setup.exe").category, "installer");
  assert.equal(categorizeLink("https://example.com/photo.webp").category, "image");
  assert.equal(categorizeLink("https://example.com/book.epub").category, "doc");
  assert.equal(categorizeLink("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567").category, "magnet");
  assert.equal(categorizeLink("https://example.com/page").category, "other");
});
