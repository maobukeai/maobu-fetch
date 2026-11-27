import {
  PLAYBACK_RATES,
  SUBTITLE_COLOR_PRESETS,
  computeSeekHover,
  formatPlaybackRateLabel,
  formatPlayerTime,
} from "./player-ui-model";

const assert = {
  equal<T>(actual: T, expected: T, message?: string) {
    if (actual !== expected) {
      throw new Error(
        `${message ? `${message}: ` : ""}expected ${String(expected)}, got ${String(actual)}`
      );
    }
  },
  deepEqual(actual: unknown, expected: unknown, message?: string) {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(
        `${message ? `${message}: ` : ""}expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`
      );
    }
  },
};

function runTests() {
  // 1. Constants check
  assert.equal(PLAYBACK_RATES.length, 7, "should expose 7 playback rates");
  assert.equal(SUBTITLE_COLOR_PRESETS.length, 5, "should expose 5 subtitle colors");

  // 2. formatPlayerTime edge cases
  assert.equal(formatPlayerTime(0), "00:00", "zero seconds");
  assert.equal(formatPlayerTime(-12), "00:00", "negative seconds");
  assert.equal(formatPlayerTime(Number.NaN, true), "00:00:00", "NaN with forceHours");
  assert.equal(formatPlayerTime(125), "02:05", "under 1h formatting");
  assert.equal(formatPlayerTime(3661), "01:01:01", "over 1h formatting");
  assert.equal(formatPlayerTime(95, true), "00:01:35", "forceHours formatting");

  // 3. formatPlaybackRateLabel
  assert.equal(formatPlaybackRateLabel(1.0), "倍速", "default rate label");
  assert.equal(formatPlaybackRateLabel(1.25), "1.25x", "1.25x rate label");
  assert.equal(formatPlaybackRateLabel(2.0), "2x", "2x rate label");

  // 4. computeSeekHover fixes rectLeft offset & clamps tooltip edges
  assert.deepEqual(
    computeSeekHover(220, 20, 400, 100),
    { ratio: 0.5, clampedX: 200, time: 50 },
    "midpoint cancels rectLeft=20 cleanly"
  );
  assert.deepEqual(
    computeSeekHover(24, 20, 400, 100, 28),
    { ratio: 0.01, clampedX: 28, time: 1 },
    "near left edge clamps tooltipX to tooltipHalfWidth"
  );
  assert.deepEqual(
    computeSeekHover(418, 20, 400, 100, 28),
    { ratio: 0.995, clampedX: 372, time: 99.5 },
    "near right edge clamps tooltipX to rectWidth - tooltipHalfWidth"
  );
  assert.deepEqual(
    computeSeekHover(-50, 20, 400, 100, 28),
    { ratio: 0, clampedX: 28, time: 0 },
    "past left edge clamps ratio to 0"
  );
  assert.deepEqual(
    computeSeekHover(100, 20, 0, 100),
    { ratio: 0, clampedX: 0, time: 0 },
    "zero-width track returns safe zeros"
  );
  assert.deepEqual(
    computeSeekHover(220, 20, 400, 0),
    { ratio: 0.5, clampedX: 200, time: 0 },
    "zero-duration keeps ratio and clampedX without NaN time"
  );

  console.log("player-ui-model.test.ts: all assertions passed");
}

runTests();
