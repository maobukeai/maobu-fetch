export interface RelativeBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type CometPlan =
  | {
      kind: "same-col";
      oldLeft: number;
      oldTop: number;
      oldHeight: number;
      midLeft: number;
      midTop: number;
      midHeight: number;
      midOffset: number;
      finalLeft: number;
      finalTop: number;
      finalHeight: number;
    }
  | {
      kind: "cross-col";
      oldLeft: number;
      oldTop: number;
      oldHeight: number;
      midLeft: number;
      midTop: number;
      midHeight: number;
      midOffset: number;
      finalLeft: number;
      finalTop: number;
      finalHeight: number;
    }
  | {
      kind: "static";
      finalLeft: number;
      finalTop: number;
      finalHeight: number;
    };

export interface OffsetNodeLike {
  offsetLeft: number;
  offsetTop: number;
  offsetWidth: number;
  offsetHeight: number;
  offsetParent: OffsetNodeLike | null;
}

/**
 * 沿 offsetParent 链向上累加至 container，返回不受 CSS transform (:active scale)
 * 影响、且天然对齐滚动容器内部坐标系的布局矩形；若链路未命中 container 返回 null。
 */
export function measureOffsetBox(
  container: OffsetNodeLike,
  item: OffsetNodeLike
): RelativeBox | null {
  let x = 0;
  let y = 0;
  let cur: OffsetNodeLike | null = item;
  let hops = 0;
  while (cur && cur !== container && hops < 32) {
    x += cur.offsetLeft;
    y += cur.offsetTop;
    cur = cur.offsetParent;
    hops += 1;
  }
  if (cur !== container) return null;
  return {
    x,
    y,
    w: item.offsetWidth,
    h: item.offsetHeight,
  };
}

/**
 * 计算目标按钮相对于滚动/定位容器内边距原点（含滚动偏移、扣除边框）的矩形坐标。
 */
export function measureItemBox(
  containerRect: Pick<DOMRect, "left" | "top">,
  itemRect: Pick<DOMRect, "left" | "top" | "width" | "height">,
  scrollLeft = 0,
  scrollTop = 0,
  clientLeft = 0,
  clientTop = 0
): RelativeBox {
  return {
    x: itemRect.left - containerRect.left - clientLeft + scrollLeft,
    y: itemRect.top - containerRect.top - clientTop + scrollTop,
    w: itemRect.width,
    h: itemRect.height,
  };
}

/**
 * 规划改版 B（流光水滴短轨 / Win11 Bounded Comet）3 关键帧轨迹：
 * - 同列上下切换：蓝竖轨做有上限（maxStretch=34px）的短水滴拖尾牵引，消除满跨度长拉生硬感；
 * - 跨单列 ↔ 2 列切换：中段收短为 6px 轻点随背板斜向滑移，落位新列左缘后舒展至目标高度；
 * - 首次或同位微调：直接返回 static 静止落位。
 */
export function computeCometPlan(
  prevBox: RelativeBox | null,
  nextBox: RelativeBox,
  railHeight = Math.min(16, Math.max(12, Math.round(nextBox.h * 0.55))),
  insetLeft = 2,
  maxStretch = 34
): CometPlan {
  const safeRailH = Math.max(4, Math.min(railHeight, nextBox.h));
  const finalTop = nextBox.y + Math.max(0, (nextBox.h - safeRailH) / 2);
  const finalLeft = nextBox.x + insetLeft;

  if (!prevBox || prevBox.w <= 0 || prevBox.h <= 0) {
    return {
      kind: "static",
      finalLeft,
      finalTop,
      finalHeight: safeRailH,
    };
  }

  const prevRailH = Math.max(4, Math.min(railHeight, prevBox.h));
  const oldTop = prevBox.y + Math.max(0, (prevBox.h - prevRailH) / 2);
  const oldLeft = prevBox.x + insetLeft;
  const dy = finalTop - oldTop;

  if (Math.abs(oldLeft - finalLeft) < 4) {
    if (Math.abs(dy) <= 4) {
      return {
        kind: "static",
        finalLeft,
        finalTop,
        finalHeight: safeRailH,
      };
    }
    const midHeight = Math.min(
      maxStretch,
      Math.max(safeRailH, safeRailH + Math.abs(dy) * 0.35)
    );
    const midTop =
      dy > 0 ? oldTop + dy * 0.45 : finalTop + Math.abs(dy) * 0.15;
    return {
      kind: "same-col",
      oldLeft,
      oldTop,
      oldHeight: prevRailH,
      midLeft: finalLeft,
      midTop: Math.round(midTop * 10) / 10,
      midHeight: Math.round(midHeight * 10) / 10,
      midOffset: 0.38,
      finalLeft,
      finalTop,
      finalHeight: safeRailH,
    };
  }

  return {
    kind: "cross-col",
    oldLeft,
    oldTop,
    oldHeight: prevRailH,
    midLeft: Math.round(((oldLeft + finalLeft) / 2) * 10) / 10,
    midTop: Math.round(((oldTop + finalTop) / 2) * 10) / 10,
    midHeight: Math.min(6, safeRailH),
    midOffset: 0.45,
    finalLeft,
    finalTop,
    finalHeight: safeRailH,
  };
}
