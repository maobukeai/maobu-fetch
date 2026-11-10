/**
 * 跳过片头片尾与剧集识别纯逻辑单元测试 (SkipIntroOutro.test.ts)
 */

declare const process: { exitCode: number; argv: string[] };

import {
  canSkipIntro,
  canSkipOutro,
  getSeriesKey,
  isEpisodeLike,
  resolveSkipSettings,
} from "./SkipIntroOutroManager";
import type { SkipIntroOutroConfig } from "./SkipTypes";

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertTrue(value: boolean, message: string): void {
  if (!value) {
    throw new Error(`${message}: expected true, got false`);
  }
}

function assertFalse(value: boolean, message: string): void {
  if (value) {
    throw new Error(`${message}: expected false, got true`);
  }
}

type TestCase = { name: string; fn: () => void };
const tests: TestCase[] = [];
const test = (name: string, fn: () => void) => {
  tests.push({ name, fn });
};

const mockConfig: SkipIntroOutroConfig = {
  enabled: true,
  autoDetect: true,
  defaultIntroSeconds: 90,
  defaultOutroSeconds: 85,
  autoPlayNextOnOutro: true,
  seriesSettings: {
    "folder:Frieren": {
      introSeconds: 95,
      outroSeconds: 90,
      updatedAt: 1700000000,
    },
  },
};

test("getSeriesKey: 正确从父目录或文件名提取剧集归属", () => {
  // 1. 父目录提取
  assertEqual(
    getSeriesKey("D:/Anime/Frieren/S01E01.mp4"),
    "folder:Frieren",
    "anime folder extraction"
  );
  assertEqual(
    getSeriesKey("C:\\Downloads\\电视剧\\庆余年第二季\\EP05.mkv"),
    "folder:庆余年第二季",
    "windows path series extraction"
  );

  // 2. 无特殊父目录时从文件名清洗
  const key1 = getSeriesKey("C:/Downloads/[Moe] Frieren - 02 [1080P].mp4");
  assertTrue(key1.includes("Frieren"), "filename series extraction");

  // 3. 根目录兜底
  assertEqual(getSeriesKey(""), "default_series", "fallback to default");
});

test("isEpisodeLike: 正确识别剧集特征与看剧模式", () => {
  // 列表多于1个视频，自动认定看剧模式
  assertTrue(isEpisodeLike("sample.mp4", 5), "multiple items in playlist");

  // 常见剧集命名模式
  assertTrue(isEpisodeLike("Frieren.S01E03.1080p.mkv", 1), "S01E03 format");
  assertTrue(isEpisodeLike("凡人修仙传_EP102_4K.mp4", 1), "EP102 format");
  assertTrue(isEpisodeLike("庆余年 第二季 第08集.mp4", 1), "chinese episode format");
  assertTrue(isEpisodeLike("[ANi] 鬼灭之刃 [01] [1080P].mp4", 1), "bracket format");

  // 普通单一视频/电影
  assertFalse(isEpisodeLike("Inception.2010.BluRay.mp4", 1), "single movie");
  assertFalse(isEpisodeLike("MyHomeVideo.mp4", 1), "single home video");
});

test("canSkipIntro: 边界与跳过片头条件判定", () => {
  const dur = 1440; // 24分钟典型剧集

  // 1. 开头 0~2秒 且满足时长，应当跳过
  assertTrue(canSkipIntro(0, 90, dur, false), "start at 0s");
  assertTrue(canSkipIntro(1.5, 90, dur, false), "start at 1.5s");

  // 2. 超过3秒后（用户已在看正片），不应突兀跳过
  assertFalse(canSkipIntro(5, 90, dur, false), "at 5s");
  assertFalse(canSkipIntro(100, 90, dur, false), "at 100s");

  // 3. 已跳过标记为 true，不再重复跳过
  assertFalse(canSkipIntro(0, 90, dur, true), "already skipped");

  // 4. 短视频（如时长仅120秒，片头90秒），保护机制不予跳过
  assertFalse(canSkipIntro(0, 90, 120, false), "short video");

  // 5. 片头秒数为0时不跳过
  assertFalse(canSkipIntro(0, 0, dur, false), "zero intro");
});

test("canSkipOutro: 边界与跳过片尾条件判定", () => {
  const dur = 1440; // 24分钟
  const outro = 85;

  // 1. 恰好进入片尾区间（剩余时间 <= 85s 且 > 1s）
  assertTrue(canSkipOutro(dur - 84, outro, dur, false), "within outro region");
  assertTrue(canSkipOutro(dur - 10, outro, dur, false), "near end");

  // 2. 还在正片中（剩余时间 > 85s），不跳过
  assertFalse(canSkipOutro(dur - 120, outro, dur, false), "in main content");

  // 3. 已经快播完了（剩余 <= 1s），自然结束即可
  assertFalse(canSkipOutro(dur - 0.5, outro, dur, false), "final second");

  // 4. 已跳过标记为 true，不再重复跳过
  assertFalse(canSkipOutro(dur - 50, outro, dur, true), "already skipped outro");

  // 5. 短视频保护
  assertFalse(canSkipOutro(50, 40, 100, false), "short video outro");
});

test("resolveSkipSettings: 优先级与决策正确", () => {
  // 1. 全局禁用
  const disabledRes = resolveSkipSettings(
    "D:/Anime/Frieren/01.mp4",
    "01.mp4",
    { ...mockConfig, enabled: false }
  );
  assertEqual(disabledRes.enabled, false, "disabled enabled flag");
  assertEqual(disabledRes.source, "disabled", "disabled source");

  // 2. 视频内置章节优先级最高
  const chapterRes = resolveSkipSettings(
    "D:/Anime/Frieren/01.mp4",
    "01.mp4",
    mockConfig,
    1,
    { introSeconds: 88, outroSeconds: 88, introTitle: "OP - 勇者", outroTitle: "ED" }
  );
  assertEqual(chapterRes.source, "chapter", "chapter source");
  assertEqual(chapterRes.introSeconds, 88, "chapter intro");
  assertEqual(chapterRes.outroSeconds, 88, "chapter outro");

  // 3. 剧集专属记忆命中
  const seriesRes = resolveSkipSettings(
    "D:/Anime/Frieren/02.mp4",
    "02.mp4",
    mockConfig,
    1
  );
  assertEqual(seriesRes.source, "series", "series source");
  assertEqual(seriesRes.introSeconds, 95, "series intro");
  assertEqual(seriesRes.outroSeconds, 90, "series outro");

  // 4. 无记忆无章节时回退到默认设置
  const defaultRes = resolveSkipSettings(
    "D:/Other/Movie/01.mp4",
    "01.mp4",
    mockConfig,
    1
  );
  assertEqual(defaultRes.source, "default", "default fallback");
  assertEqual(defaultRes.introSeconds, 90, "default intro");
  assertEqual(defaultRes.outroSeconds, 85, "default outro");
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
  console.error(`\n${failed} skip intro outro tests failed`);
  process.exitCode = 1;
} else {
  console.log(`\nPassed ${tests.length} / ${tests.length} skip intro outro tests.\n`);
}
