import test from "node:test";
import assert from "node:assert/strict";
import { bridgeMediaTask, selectBridgeMediaFormat } from "./media-selection.js";

test("browser bridge prefers a lightweight combined format", () => {
  const formats = [
    { id: "bestvideo+bestaudio", has_video: true, has_audio: true, requires_ffmpeg: true },
    { id: "18", has_video: true, has_audio: true, requires_ffmpeg: false, extension: "mp4" },
  ];
  assert.equal(selectBridgeMediaFormat({ formats }).id, "18");
});

test("browser bridge preserves the selected component requirement", () => {
  const task = bridgeMediaTask({
    title: "示例:视频",
    formats: [{ id: "high", label: "最高画质", extension: "mp4", has_video: true, has_audio: true, requires_ffmpeg: true }],
  });
  assert.equal(task.fileName, "示例_视频.mp4");
  assert.equal(task.media.requires_ffmpeg, true);
});

test("browser bridge refuses DRM media", () => {
  assert.throws(() => bridgeMediaTask({ drm: true, formats: [] }), /DRM/);
});
