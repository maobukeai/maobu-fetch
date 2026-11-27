import {
  computeCometPlan,
  measureItemBox,
  measureOffsetBox,
  type OffsetNodeLike,
  type RelativeBox,
} from "./geometry";

const assert = {
  ok(cond: boolean, message?: string) {
    if (!cond) throw new Error(message ?? "Expected condition to be truthy");
  },
  equal(actual: unknown, expected: unknown, message?: string) {
    if (actual !== expected) {
      throw new Error(
        message ?? `Expected ${String(expected)}, got ${String(actual)}`
      );
    }
  },
  deepEqual(actual: unknown, expected: unknown) {
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) {
      throw new Error(`Expected ${b}, got ${a}`);
    }
  },
};

// 1. measureOffsetBox 沿 offsetParent 链准确累加，且未命中时返回 null
const container: OffsetNodeLike = {
  offsetLeft: 0,
  offsetTop: 0,
  offsetWidth: 184,
  offsetHeight: 500,
  offsetParent: null,
};
const gridWrapper: OffsetNodeLike = {
  offsetLeft: 6,
  offsetTop: 220,
  offsetWidth: 170,
  offsetHeight: 110,
  offsetParent: container,
};
const gridBtn: OffsetNodeLike = {
  offsetLeft: 86,
  offsetTop: 28,
  offsetWidth: 80,
  offsetHeight: 26,
  offsetParent: gridWrapper,
};
const detachedBtn: OffsetNodeLike = {
  offsetLeft: 10,
  offsetTop: 10,
  offsetWidth: 50,
  offsetHeight: 20,
  offsetParent: null,
};

assert.deepEqual(measureOffsetBox(container, gridBtn), {
  x: 92,
  y: 248,
  w: 80,
  h: 26,
});
assert.deepEqual(measureOffsetBox(container, detachedBtn), null);

// 2. measureItemBox 含滚动与边框修正
assert.deepEqual(
  measureItemBox(
    { left: 100, top: 200 },
    { left: 112, top: 248, width: 160, height: 28 },
    0,
    40,
    1,
    1
  ),
  { x: 11, y: 87, w: 160, h: 28 }
);

// 3. 首次定位（无 prevBox）返回 static
const row0: RelativeBox = { x: 8, y: 20, w: 168, h: 29 };
assert.deepEqual(computeCometPlan(null, row0, 16, 2, 34), {
  kind: "static",
  finalLeft: 10,
  finalTop: 26.5,
  finalHeight: 16,
});

// 4. 同列长距离向下切换：拉伸高度受 maxStretch=34px 严格封顶
const row5: RelativeBox = { x: 8, y: 220, w: 168, h: 29 };
const downPlan = computeCometPlan(row0, row5, 16, 2, 34);
assert.equal(downPlan.kind, "same-col");
if (downPlan.kind === "same-col") {
  assert.equal(downPlan.midHeight, 34);
  assert.equal(downPlan.midOffset, 0.38);
  assert.equal(downPlan.midTop, 116.5);
  assert.equal(downPlan.finalTop, 226.5);
  assert.equal(downPlan.finalHeight, 16);
}

// 5. 同列短距离向上切换：按实际距离 35% 温和拉伸，不超过 34px
const row1: RelativeBox = { x: 8, y: 50, w: 168, h: 29 };
const upPlan = computeCometPlan(row1, row0, 16, 2, 34);
assert.equal(upPlan.kind, "same-col");
if (upPlan.kind === "same-col") {
  assert.equal(upPlan.midHeight, 26.5);
  assert.ok(upPlan.midHeight <= 34);
  assert.equal(upPlan.midTop, 31);
  assert.equal(upPlan.finalTop, 26.5);
}

// 6. 单列切到 2 列网格右列：返回 cross-col 且中段收短为 6px 轻点
const gridRight: RelativeBox = { x: 96, y: 260, w: 80, h: 26 };
const crossPlan = computeCometPlan(row0, gridRight, 14, 2, 34);
assert.deepEqual(crossPlan, {
  kind: "cross-col",
  oldLeft: 10,
  oldTop: 27.5,
  oldHeight: 14,
  midLeft: 54,
  midTop: 146.8,
  midHeight: 6,
  midOffset: 0.45,
  finalLeft: 98,
  finalTop: 266,
  finalHeight: 14,
});

console.log("PASS magnetic-switch geometry tests");
