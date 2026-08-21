/**
 * 字幕解析单元测试（SubtitleParser.test.ts）。
 * 挂载在 pnpm run check 中。
 */

declare const process: { exitCode: number; argv: string[] };

import { getActiveCueText, parseAnySubtitles, parseAssSubtitles, parseSubtitles, parseTimestamp } from "./SubtitleParser.js";

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertClose(actual: number, expected: number, delta = 0.01, message = ""): void {
  if (Math.abs(actual - expected) > delta) {
    throw new Error(`${message}: expected ~${expected}, got ${actual}`);
  }
}

type TestCase = { name: string; fn: () => void };
const tests: TestCase[] = [];
const test = (name: string, fn: () => void) => { tests.push({ name, fn }); };

test("时间戳解析正确", () => {
  assertClose(parseTimestamp("00:01:23.456"), 83.456, 0.001, "00:01:23.456");
  assertClose(parseTimestamp("00:01:23,456"), 83.456, 0.001, "00:01:23,456");
  assertClose(parseTimestamp("02:30"), 150, 0.001, "02:30");
});

test("解析标准 SRT 字幕格式", () => {
  const srt = `
1
00:00:01,000 --> 00:00:04,000
你好，猫步下载器！

2
00:00:05,500 --> 00:00:08,200
轻量极速多媒体播放。
`;
  const cues = parseSubtitles(srt);
  assertEqual(cues.length, 2, "cues count");
  assertEqual(cues[0].start, 1.0, "cue 0 start");
  assertEqual(cues[0].end, 4.0, "cue 0 end");
  assertEqual(cues[0].text, "你好，猫步下载器！", "cue 0 text");
  assertEqual(cues[1].start, 5.5, "cue 1 start");
  assertEqual(cues[1].end, 8.2, "cue 1 end");
});

test("解析 WebVTT 格式字幕", () => {
  const vtt = `WEBVTT

1
00:00:02.000 --> 00:00:05.000 position:50% line:0
<i>Hello World</i>
`;
  const cues = parseSubtitles(vtt);
  assertEqual(cues.length, 1, "cues count");
  assertEqual(cues[0].start, 2.0, "cue 0 start");
  assertEqual(cues[0].end, 5.0, "cue 0 end");
  assertEqual(cues[0].text, "Hello World", "cue 0 text");
});

test("解析 ASS / SSA 格式字幕并清洗样式代码", () => {
  const ass = `[Script Info]
Title: Test
[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.20,0:00:04.50,Default,,0,0,0,,{\\pos(192,200)\\c&H00ffff&}第一句ASS字幕\\N第二行
Dialogue: 0,0:00:06.00,0:00:08.00,Default,,0,0,0,,{\\an8}第二句字幕
`;
  const cues = parseAssSubtitles(ass);
  assertEqual(cues.length, 2, "ass cues count");
  assertClose(cues[0].start, 1.20, 0.01, "ass cue 0 start");
  assertClose(cues[0].end, 4.50, 0.01, "ass cue 0 end");
  assertEqual(cues[0].text, "第一句ASS字幕\n第二行", "ass cue 0 cleaned text");
  assertClose(cues[1].start, 6.00, 0.01, "ass cue 1 start");
  assertEqual(cues[1].text, "第二句字幕", "ass cue 1 text");
});

test("parseAnySubtitles 门面智能按扩展名或内容分流", () => {
  const assContent = `Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,通用门面解析测试`;
  const srtContent = `1\n00:00:01,000 --> 00:00:03,000\nSRT通用门面测试`;

  const cuesAss = parseAnySubtitles(assContent, "ass");
  assertEqual(cuesAss.length, 1, "ass count");
  assertEqual(cuesAss[0].text, "通用门面解析测试", "ass text");

  const cuesSrt = parseAnySubtitles(srtContent, "srt");
  assertEqual(cuesSrt.length, 1, "srt count");
  assertEqual(cuesSrt[0].text, "SRT通用门面测试", "srt text");
});

test("根据播放进度命中字幕与时轴偏移", () => {
  const srt = `
1
00:00:01,000 --> 00:00:04,000
第一句字幕

2
00:00:06,000 --> 00:00:09,000
第二句字幕
`;
  const cues = parseSubtitles(srt);
  assertEqual(getActiveCueText(cues, 0.5), "", "before cue 1");
  assertEqual(getActiveCueText(cues, 2.5), "第一句字幕", "inside cue 1");
  assertEqual(getActiveCueText(cues, 5.0), "", "between cues");
  assertEqual(getActiveCueText(cues, 7.5), "第二句字幕", "inside cue 2");
  assertEqual(getActiveCueText(cues, 0.5, 1.0), "第一句字幕", "with offset");
});

let failed = 0;
for (const t of tests) {
  try {
    t.fn();
    console.log(`✓ ${t.name}`);
  } catch (e) {
    failed++;
    console.error(`✗ ${t.name}: ${(e as Error).message}`);
  }
}

if (failed > 0) {
  console.error(`\n${failed} tests failed`);
  process.exitCode = 1;
} else {
  console.log(`\nPassed ${tests.length} / ${tests.length} subtitle parser tests.\n`);
}
