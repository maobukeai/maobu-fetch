import {
  computeModalOriginVector,
  isOriginFresh,
  MODAL_BACKDROP_OPEN_MS,
  MODAL_CLOSE_MS,
  MODAL_CLOSE_SCALE,
  MODAL_FALLBACK_OFFSET_Y,
  MODAL_OPEN_MS,
  MODAL_OPEN_SCALE,
  MODAL_ORIGIN_FRESH_MS,
} from "./geometry";

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

// 1. 时序与缩放常量校验
assert.equal(MODAL_OPEN_MS, 220);
assert.equal(MODAL_BACKDROP_OPEN_MS, 190);
assert.equal(MODAL_CLOSE_MS, 165);
assert.equal(MODAL_OPEN_SCALE, 0.88);
assert.equal(MODAL_CLOSE_SCALE, 0.9);
assert.equal(MODAL_ORIGIN_FRESH_MS, 650);

// 2. 时效判断：650ms 内有效，过期或负数无效
assert.equal(isOriginFresh(null, 1000), false);
assert.equal(isOriginFresh(1000, 1000), true);
assert.equal(isOriginFresh(1000, 1650), true);
assert.equal(isOriginFresh(1000, 1651), false);
assert.equal(isOriginFresh(2000, 1500), false);

// 3. 无触发点 / 非法视口安全回退
const fallback = computeModalOriginVector(null, { width: 1280, height: 720 });
assert.equal(fallback.ox, 0);
assert.equal(fallback.oy, MODAL_FALLBACK_OFFSET_Y);
assert.equal(fallback.isPointerOrigin, false);

const badViewport = computeModalOriginVector({ x: 100, y: 100 }, { width: 0, height: 720 });
assert.equal(badViewport.isPointerOrigin, false);
assert.equal(badViewport.oy, MODAL_FALLBACK_OFFSET_Y);

// 4. 象限向量方向校验（左上 -> 负 ox, 负 oy；右上 -> 正 ox, 负 oy）
const topLeft = computeModalOriginVector({ x: 92, y: 74 }, { width: 1280, height: 720 });
assert.equal(topLeft.isPointerOrigin, true);
assert.ok(topLeft.ox < 0 && topLeft.oy < 0, "Top-left trigger should have negative ox/oy");

const topRight = computeModalOriginVector({ x: 1080, y: 40 }, { width: 1280, height: 720 });
assert.equal(topRight.isPointerOrigin, true);
assert.ok(topRight.ox > 0 && topRight.oy < 0, "Top-right trigger should have positive ox and negative oy");

// 5. 超宽屏/超高屏向量钳位边界 [-220, 220] / [-140, 140]
const extremeMin = computeModalOriginVector({ x: 0, y: 0 }, { width: 4000, height: 3000 });
assert.equal(extremeMin.ox, -220);
assert.equal(extremeMin.oy, -140);

const extremeMax = computeModalOriginVector({ x: 4000, y: 3000 }, { width: 4000, height: 3000 });
assert.equal(extremeMax.ox, 220);
assert.equal(extremeMax.oy, 140);

console.log("modal-motion geometry.test.ts: ALL PASS");
