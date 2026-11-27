import {
  computeSpeedSparkGeometry,
  DETAILS_DRAWER_CLOSE_MS,
  DETAILS_DRAWER_OPEN_MS,
  getDetailsDrawerUnmountDelay,
  getDetailsStatusTone,
  getTabSlideDirection,
} from "./details-motion";

const assert = {
  equal(actual: unknown, expected: unknown, message?: string) {
    if (actual !== expected) {
      throw new Error(
        message ?? `Expected ${String(expected)}, got ${String(actual)}`
      );
    }
  },
  ok(cond: unknown, message?: string) {
    if (!cond) {
      throw new Error(message ?? "Expected truthy value");
    }
  },
};

// 1. Tab 方向感知判定
assert.equal(getTabSlideDirection("info", "diagnosis"), "right");
assert.equal(getTabSlideDirection("info", "connections"), "right");
assert.equal(getTabSlideDirection("diagnosis", "precheck"), "right");
assert.equal(getTabSlideDirection("connections", "info"), "left");
assert.equal(getTabSlideDirection("precheck", "diagnosis"), "left");
assert.equal(getTabSlideDirection("info", "info"), "none");

// 2. 状态语义色调判定
assert.equal(getDetailsStatusTone("downloading"), "accent");
assert.equal(getDetailsStatusTone("downloading", true), "accent");
assert.equal(getDetailsStatusTone("completed"), "success");
assert.equal(getDetailsStatusTone("failed"), "danger");
assert.equal(getDetailsStatusTone("interrupted"), "danger");
assert.equal(getDetailsStatusTone("remote-changed"), "danger");
assert.equal(getDetailsStatusTone("paused"), "warning");
assert.equal(getDetailsStatusTone("paused-by-metered"), "warning");
assert.equal(getDetailsStatusTone("paused-by-low-disk"), "warning");
assert.equal(getDetailsStatusTone("waiting-network"), "warning");
assert.equal(getDetailsStatusTone("queued"), "warning");
assert.equal(getDetailsStatusTone("scheduled"), "warning");
assert.equal(getDetailsStatusTone("cancelled"), "muted");

// 3. 实时速度曲线几何与脉冲尾点计算
const emptyGeo = computeSpeedSparkGeometry([], 280, 48);
assert.equal(emptyGeo.peak, 0);
assert.equal(emptyGeo.latest, 0);
assert.equal(emptyGeo.linePath, "");
assert.equal(emptyGeo.areaPath, "");
assert.equal(emptyGeo.isLive, false);

const singleGeo = computeSpeedSparkGeometry([1024], 280, 48);
assert.equal(singleGeo.peak, 1024);
assert.equal(singleGeo.latest, 1024);
assert.equal(singleGeo.linePath, "");
assert.equal(singleGeo.isLive, true);
assert.ok(singleGeo.lastYPercent >= 0 && singleGeo.lastYPercent <= 100);

const liveGeo = computeSpeedSparkGeometry([100, 300, 200, 400], 280, 48);
assert.equal(liveGeo.peak, 400);
assert.equal(liveGeo.latest, 400);
assert.equal(liveGeo.isLive, true);
assert.ok(liveGeo.linePath.startsWith("M "));
assert.ok(liveGeo.areaPath.endsWith(" L 280,48 L 0,48 Z"));
assert.ok(liveGeo.lastYPercent >= 0 && liveGeo.lastYPercent <= 100);

const stoppedGeo = computeSpeedSparkGeometry([100, 400, 0], 280, 48);
assert.equal(stoppedGeo.peak, 400);
assert.equal(stoppedGeo.latest, 0);
assert.equal(stoppedGeo.isLive, false);
assert.ok(stoppedGeo.lastYPercent >= 0 && stoppedGeo.lastYPercent <= 100);

// 4. 抽屉展开/收缩时序常量与卸载延迟判定
assert.equal(DETAILS_DRAWER_OPEN_MS, 210);
assert.equal(DETAILS_DRAWER_CLOSE_MS, 170);
assert.equal(getDetailsDrawerUnmountDelay(false), 190);
assert.equal(getDetailsDrawerUnmountDelay(true), 0);

console.log("details-motion.test.ts: ALL PASS");
