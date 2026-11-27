import {
  SPEED_LIMIT_PRESETS,
  clampMenuPosition,
  formatSpeedLimitBadge,
  validateRenameFilename,
} from "./light-modals-model";

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
  // 1. Speed limit presets sanity
  assert.equal(SPEED_LIMIT_PRESETS.length, 6, "should expose 6 speed presets");
  assert.equal(SPEED_LIMIT_PRESETS[0]?.kb, 0, "first preset should be unlimited");
  assert.equal(
    SPEED_LIMIT_PRESETS[0]?.labelKey,
    "details.noSpeedLimit",
    "unlimited preset should reference i18n key"
  );

  // 2. formatSpeedLimitBadge formatting
  assert.equal(formatSpeedLimitBadge(0, "不限速"), "不限速", "0 bytes returns unlimited");
  assert.equal(formatSpeedLimitBadge(-100, "Unlimited"), "Unlimited", "negative returns unlimited");
  assert.equal(formatSpeedLimitBadge(Number.NaN, "Unlimited"), "Unlimited", "NaN returns unlimited");
  assert.equal(formatSpeedLimitBadge(512 * 1024, "不限速"), "512 KB/s", "512 KB/s formatting");
  assert.equal(formatSpeedLimitBadge(2048 * 1024, "不限速"), "2 MB/s", "whole MB/s formatting");
  assert.equal(formatSpeedLimitBadge(1536 * 1024, "不限速"), "1.5 MB/s", "fractional MB/s formatting");

  // 3. validateRenameFilename validation
  assert.equal(validateRenameFilename("   "), "empty", "blank filename rejected");
  assert.equal(validateRenameFilename(".."), "path-traversal", "dot-dot rejected");
  assert.equal(validateRenameFilename("."), "path-traversal", "single dot rejected");
  assert.equal(validateRenameFilename("../secret.zip"), "path-traversal", "parent traversal rejected");
  assert.equal(validateRenameFilename("dir\\..\\file.exe"), "path-traversal", "backslash traversal rejected");
  assert.equal(validateRenameFilename("bad:name?.zip"), "invalid-chars", "Windows invalid chars rejected");
  assert.equal(validateRenameFilename("folder/file.zip"), "invalid-chars", "path separator rejected");
  assert.equal(validateRenameFilename("a".repeat(241) + ".zip"), "too-long", "overlong filename rejected");
  assert.equal(validateRenameFilename("Ubuntu-24.04..desktop-amd64.iso"), null, "valid filename with inner dots accepted");

  // 4. clampMenuPosition viewport edge flipping & transform-origin
  assert.deepEqual(
    clampMenuPosition(120, 160, 228, 300, 1280, 720),
    { left: 120, top: 160, originX: "left", originY: "top" },
    "in-bounds menu stays anchored top-left"
  );
  assert.deepEqual(
    clampMenuPosition(1250, 680, 228, 300, 1280, 720),
    { left: 1022, top: 380, originX: "right", originY: "bottom" },
    "bottom-right overflow flips origin to right bottom"
  );
  assert.deepEqual(
    clampMenuPosition(2, 3, 228, 300, 1280, 720),
    { left: 8, top: 8, originX: "left", originY: "top" },
    "top-left edge clamps to padding"
  );
  assert.deepEqual(
    clampMenuPosition(50, 50, 400, 400, 320, 240),
    { left: 8, top: 8, originX: "right", originY: "bottom" },
    "oversized menu on tiny viewport stays safely clamped"
  );

  console.log("light-modals-model.test.ts: all assertions passed");
}

runTests();
