import test from "node:test";
import assert from "node:assert/strict";
import { createHash, createHmac, webcrypto } from "node:crypto";
import { signature } from "./protocol.js";

globalThis.crypto ??= webcrypto;

test("bridge signature matches the Rust HMAC protocol", async () => {
  const token = "0123456789abcdef";
  const timestamp = "1784419200000";
  const body = JSON.stringify({ url: "https://example.com/file.zip" });
  const key = createHash("sha256").update(token).digest();
  const expected = createHmac("sha256", key).update(`${timestamp}\n${body}`).digest("hex");
  assert.equal(await signature(token, timestamp, body), expected);
});
