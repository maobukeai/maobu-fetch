import { createHash, createHmac } from "node:crypto";

const destination = process.argv[2];
if (!destination) throw new Error("Usage: node scripts/range_e2e.mjs <download-directory>");

const extension = "abcdefghijklmnopabcdefghijklmnop";
const token = "lumaget-e2e-token";
const key = createHash("sha256").update(token).digest();
const body = JSON.stringify({
  url: "http://127.0.0.1:18765/fixture.bin",
  file_name: "fixture.bin",
  destination,
  source: "e2e",
  connection_count: 8,
  collision_policy: "overwrite",
});
const timestamp = Date.now().toString();
const signature = createHmac("sha256", key).update(`${timestamp}\n${body}`).digest("hex");
const response = await fetch("http://127.0.0.1:17433/v1/tasks", {
  method: "POST",
  headers: {
    "content-type": "application/json",
    origin: `chrome-extension://${extension}`,
    "x-luma-extension": extension,
    "x-luma-timestamp": timestamp,
    "x-luma-signature": signature,
  },
  body,
});
const text = await response.text();
if (!response.ok) throw new Error(`${response.status}: ${text}`);
console.log(text);
