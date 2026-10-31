import type { DownloadTask } from "../../types";
import { canFinishRecording } from "./recording-control";
const assert = { equal(actual: unknown, expected: unknown, reason = "recording visibility") {
  if (actual !== expected) throw new Error(reason);
} };

const task = { task_kind: "http", status: "downloading", url: "https://fixture.invalid/video.m3u8", total_bytes: 0 } as DownloadTask;
assert.equal(canFinishRecording(task), false, "unknown duration or m3u8 does not prove live");
assert.equal(canFinishRecording({ ...task, media: { subtitles: [], is_live: false } }), false);
const live = { ...task, media: { subtitles: [], is_live: true } };
assert.equal(canFinishRecording(live), true);
for (const status of ["paused", "completed", "failed", "cancelled"] as const) {
  assert.equal(canFinishRecording({ ...live, status }), false);
}
assert.equal(canFinishRecording({ ...live, task_kind: "bt" }), false);
console.log("recording action visibility: 8 cases passed");
