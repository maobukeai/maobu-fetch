import test from "node:test";
import assert from "node:assert/strict";
import { cleanTrackingUrl, normalizeExtension, normalizeHost, parseRules, requestPageWithTrackingFallback } from "./rules.js";

test("normalizes and deduplicates interception rules", () => {
  const hosts = parseRules("*.Example.com\nexample.com\n例子.测试", normalizeHost);
  assert.deepEqual(hosts.values, ["example.com", "xn--fsqu00a.xn--0zwm56d"]);
  assert.deepEqual(hosts.invalid, []);
  assert.equal(normalizeExtension(".ZIP"), "zip");
  assert.throws(() => normalizeExtension("bad/type"));
});

test("cleans only known tracking parameters on share pages", () => {
  assert.equal(
    cleanTrackingUrl("https://example.com/page?id=42&utm_source=test&fbclid=abc#part"),
    "https://example.com/page?id=42#part",
  );
  const signed = "https://example.com/file?token=secret&utm_source=test";
  assert.equal(cleanTrackingUrl(signed), signed);
});

test("retries the original page URL when a cleaned probe fails", async () => {
  const calls = [];
  const response = await requestPageWithTrackingFallback(async (url) => {
    calls.push(url);
    return { ok: calls.length > 1 };
  }, "https://example.com/page?id=42&utm_source=test");
  assert.equal(response.ok, true);
  assert.deepEqual(calls, [
    "https://example.com/page?id=42",
    "https://example.com/page?id=42&utm_source=test",
  ]);
});
