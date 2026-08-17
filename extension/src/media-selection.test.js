import test from "node:test";
import assert from "node:assert/strict";
import { bridgeMediaTask, filterSubtitles, selectBridgeMediaFormat } from "./media-selection.js";

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

// ---- 清晰度偏好（P3-17）----

const QUALITY_FORMATS = [
  { id: "2160", has_video: true, has_audio: true, requires_ffmpeg: false, height: 2160 },
  { id: "1080", has_video: true, has_audio: true, requires_ffmpeg: false, height: 1080 },
  { id: "720", has_video: true, has_audio: true, requires_ffmpeg: false, height: 720 },
  { id: "audio-only", has_video: false, has_audio: true, requires_ffmpeg: false },
];

test("quality preference: 默认 best 选最高清晰度", () => {
  assert.equal(selectBridgeMediaFormat({ formats: QUALITY_FORMATS }).id, "2160");
  assert.equal(selectBridgeMediaFormat({ formats: QUALITY_FORMATS }, "best").id, "2160");
  assert.equal(selectBridgeMediaFormat({ formats: QUALITY_FORMATS }, "未知值").id, "2160");
});

test("quality preference: 1080/720 上限选不超过上限的最高清晰度", () => {
  assert.equal(selectBridgeMediaFormat({ formats: QUALITY_FORMATS }, "1080").id, "1080");
  assert.equal(selectBridgeMediaFormat({ formats: QUALITY_FORMATS }, "720").id, "720");
});

test("quality preference: 上限低于所有清晰度时回退 best，不空手而归", () => {
  const only4k = QUALITY_FORMATS.filter((f) => f.height >= 2160);
  assert.equal(selectBridgeMediaFormat({ formats: only4k }, "720").id, "2160");
});

test("quality preference: audio 优先纯音频流", () => {
  assert.equal(selectBridgeMediaFormat({ formats: QUALITY_FORMATS }, "audio").id, "audio-only");
});

test("quality preference: audio 但无纯音频流时回退最高清晰度", () => {
  const noAudioOnly = QUALITY_FORMATS.filter((f) => f.has_video);
  assert.equal(selectBridgeMediaFormat({ formats: noAudioOnly }, "audio").id, "2160");
});

test("subtitles: 探测结果的字幕语言列表透传给任务", () => {
  const task = bridgeMediaTask({
    title: "示例",
    subtitles: ["zh-Hans", "en", "ja"],
    formats: [{ id: "18", has_video: true, has_audio: true, requires_ffmpeg: false, extension: "mp4" }],
  });
  assert.deepEqual(task.media.subtitles, ["zh-Hans", "en", "ja"]);
});

test("subtitles: 探测结果缺失字幕字段时任务携带空数组", () => {
  const task = bridgeMediaTask({
    title: "示例",
    formats: [{ id: "18", has_video: true, has_audio: true, requires_ffmpeg: false, extension: "mp4" }],
  });
  assert.deepEqual(task.media.subtitles, []);
});

// ---- 字幕语言偏好（P3）----

test("subtitles preference: all 保留全部语言", () => {
  assert.deepEqual(filterSubtitles(["zh-Hans", "en", "ja"], "all"), ["zh-Hans", "en", "ja"]);
  assert.deepEqual(filterSubtitles(["zh-Hans", "en"], undefined), ["zh-Hans", "en"]);
});

test("subtitles preference: none 清空字幕列表", () => {
  assert.deepEqual(filterSubtitles(["zh-Hans", "en"], "none"), []);
});

test("subtitles preference: zh 只保留中文字幕，无中文时保留全部", () => {
  assert.deepEqual(filterSubtitles(["zh-Hans", "zh-CN", "en", "ja"], "zh"), ["zh-Hans", "zh-CN"]);
  assert.deepEqual(filterSubtitles(["en", "ja"], "zh"), ["en", "ja"]);
});

test("subtitles preference: 偏好传入 bridgeMediaTask 生效", () => {
  const probe = {
    title: "示例",
    subtitles: ["zh-Hans", "en"],
    formats: [{ id: "18", has_video: true, has_audio: true, requires_ffmpeg: false, extension: "mp4" }],
  };
  assert.deepEqual(bridgeMediaTask(probe, "示例", "best", "zh").media.subtitles, ["zh-Hans"]);
  assert.deepEqual(bridgeMediaTask(probe, "示例", "best", "none").media.subtitles, []);
});
