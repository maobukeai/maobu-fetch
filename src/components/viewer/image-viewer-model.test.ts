import {
  clampScale,
  computeCursorAnchoredZoom,
  computeViewerImageUrl,
  formatAspectRatio,
  nextFilterMode,
  VIEWER_MAX_SCALE,
  VIEWER_MIN_SCALE,
} from "./image-viewer-model";

function assert(condition: boolean, msg: string) {
  if (!condition) {
    throw new Error(`断言失败: ${msg}`);
  }
}

function assertClose(actual: number, expected: number, eps: number, msg: string) {
  if (Math.abs(actual - expected) > eps) {
    throw new Error(`断言失败: ${msg} (expected ${expected}, got ${actual})`);
  }
}

console.log("▶ 测试 image-viewer-model 纯逻辑...");

// 1. clampScale 边界与精度
assert(clampScale(0.01) === VIEWER_MIN_SCALE, "最小缩放应限制为 0.1");
assert(clampScale(15) === VIEWER_MAX_SCALE, "最大缩放应限制为 10");
assert(clampScale(1.234) === 1.23, "缩放应保留两位小数");
assert(clampScale(Number.NaN) === 1, "非法值应回退到 1");

// 2. 指针锚定缩放（中心与偏心不变量）
const rect = { left: 100, top: 50, width: 800, height: 600 }; // 中心在 (500, 350)
const centerZoom = computeCursorAnchoredZoom({
  scale: 1,
  position: { x: 0, y: 0 },
  factor: 1.5,
  pointerX: 500,
  pointerY: 350,
  stageRect: rect,
});
assert(centerZoom.scale === 1.5, "中心放大应更新比例为 1.5");
assertClose(centerZoom.position.x, 0, 0.01, "中心放大 x 偏移应保持 0");
assertClose(centerZoom.position.y, 0, 0.01, "中心放大 y 偏移应保持 0");

// 偏心点不变量：缩放前指针下方相对于中心的坐标 ix = (cx - pos.x) / scale
const px = 660; // cx = 160
const py = 230; // cy = -120
const initPos = { x: 20, y: -10 };
const initScale = 1;
const offZoom = computeCursorAnchoredZoom({
  scale: initScale,
  position: initPos,
  factor: 2,
  pointerX: px,
  pointerY: py,
  stageRect: rect,
});
assert(offZoom.scale === 2, "偏心放大比例应为 2");
const cx = px - (rect.left + rect.width / 2);
const cy = py - (rect.top + rect.height / 2);
const ixBefore = (cx - initPos.x) / initScale;
const iyBefore = (cy - initPos.y) / initScale;
const cxAfter = ixBefore * offZoom.scale + offZoom.position.x;
const cyAfter = iyBefore * offZoom.scale + offZoom.position.y;
assertClose(cxAfter, cx, 0.05, "偏心缩放后指针处 X 像素位置必须保持不变");
assertClose(cyAfter, cy, 0.05, "偏心缩放后指针处 Y 像素位置必须保持不变");

// 到达上限时位置不得漂移
const capped = computeCursorAnchoredZoom({
  scale: 10,
  position: { x: 42, y: -18 },
  factor: 1.25,
  pointerX: 700,
  pointerY: 400,
  stageRect: rect,
});
assert(capped.scale === 10 && capped.position.x === 42 && capped.position.y === -18, "到达最大缩放上限时位置不得继续漂移");

// 3. 宽高比格式化
assert(formatAspectRatio(1920, 1080) === "16:9", "1920x1080 应识别为 16:9");
assert(formatAspectRatio(1024, 1024) === "1:1", "1024x1024 应识别为 1:1");
assert(formatAspectRatio(2400, 1600) === "3:2", "2400x1600 应识别为 3:2");
assert(formatAspectRatio(1366, 768) === "1.78:1", "复杂分辨率应以小数比输出");
assert(formatAspectRatio(0, 1080) === "—", "零宽度应返回 —");

// 4. 滤镜循环切换
const f1 = nextFilterMode("none");
const f2 = nextFilterMode(f1.mode);
const f3 = nextFilterMode(f2.mode);
const f4 = nextFilterMode(f3.mode);
assert(f1.mode === "contrast", "none -> contrast");
assert(f2.mode === "grayscale", "contrast -> grayscale");
assert(f3.mode === "invert", "grayscale -> invert");
assert(f4.mode === "none", "invert -> none");

// 5. 图片 URL 解析
assert(computeViewerImageUrl("", false, true) === "", "空路径应返回空串");
assert(
  computeViewerImageUrl("data:image/svg+xml,<svg/>", false, true) === "data:image/svg+xml,<svg/>",
  "data URI 应透传"
);
assert(
  computeViewerImageUrl("C:\\img\\a.png", false, true, (p) => `asset:///${p}`) === "asset:///C:\\img\\a.png",
  "桌面端应优先使用 convertFn"
);
assert(
  computeViewerImageUrl("C:\\img\\a.png", true, true, (p) => `asset:///${p}`).startsWith("stream://localhost?file="),
  "fallback=true 时应切回 stream 协议"
);
assert(
  computeViewerImageUrl("C:\\img\\a.png", false, true, () => {
    throw new Error("boom");
  }).startsWith("stream://localhost?file="),
  "convertFn 抛出异常时应安全降级至 stream 协议"
);

console.log("✔ 全部 image-viewer-model 单元测试通过！");
