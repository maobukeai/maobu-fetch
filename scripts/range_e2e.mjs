import { createHash, createHmac } from "node:crypto";

const destination = process.argv[2];
const connections = Number(process.argv[3] ?? 32);
const sourceUrl = process.argv[4] ?? "http://127.0.0.1:18765/fixture.bin";
const fileName = process.argv[5] ?? "fixture.bin";
const collisionPolicy = process.argv[6] ?? "overwrite";
const priority = Number(process.argv[7] ?? 0);
const perTaskSpeedLimit = Number(process.argv[8] ?? 0);
if (!destination) throw new Error("Usage: node scripts/range_e2e.mjs <download-directory> [connections] [url] [file-name]");

const extension = "abcdefghijklmnopabcdefghijklmnop";
const token = "lumaget-e2e-token";
const key = createHash("sha256").update(token).digest();
const body = JSON.stringify({
  url: sourceUrl,
  file_name: fileName,
  destination,
  source: "e2e",
  connection_count: connections,
  priority,
  per_task_speed_limit: perTaskSpeedLimit,
  collision_policy: collisionPolicy,
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
