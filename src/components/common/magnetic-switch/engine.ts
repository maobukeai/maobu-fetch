import {
  computeCometPlan,
  measureItemBox,
  measureOffsetBox,
  type OffsetNodeLike,
  type RelativeBox,
} from "./geometry";

export type MagneticMode = "rail" | "pill";

const RAIL_SELECTOR = '.nav-scroll, .settings-nav-list, [data-magnetic-group="rail"]';
const PILL_SELECTOR =
  '.fluent-segmented-control, .details-tabs, .progress-window-tabs, [role="tablist"], [data-magnetic-group="pill"]';
const ANY_GROUP_SELECTOR = `${RAIL_SELECTOR}, ${PILL_SELECTOR}`;

const COMET_DURATION_MS = 245;
const COMET_EASING = "cubic-bezier(0.22, 1, 0.36, 1)";

interface ContainerRecord {
  mode: MagneticMode;
  hoverEl: HTMLDivElement;
  activeEl: HTMLDivElement;
  wormEl: HTMLDivElement | null;
  prevActiveBtn: HTMLElement | null;
  prevActiveBox: RelativeBox | null;
  initialized: boolean;
  cometAnim: Animation | null;
  rafId: number | null;
  mutObserver: MutationObserver;
  resizeObserver: ResizeObserver;
  cleanupListeners: () => void;
}

const records = new WeakMap<HTMLElement, ContainerRecord>();
const activeContainers = new Set<HTMLElement>();
let globalInstalledCleanup: (() => void) | null = null;

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function resolveMode(container: HTMLElement): MagneticMode | null {
  const explicit = container.getAttribute("data-magnetic-group");
  if (explicit === "off") return null;
  if (explicit === "rail") return "rail";
  if (explicit === "pill") return "pill";
  if (container.matches(RAIL_SELECTOR)) return "rail";
  if (container.matches(PILL_SELECTOR)) return "pill";
  return null;
}

function isNestedInsideManagedGroup(el: HTMLElement): boolean {
  let parent = el.parentElement;
  while (parent) {
    if (records.has(parent)) return true;
    parent = parent.parentElement;
  }
  return false;
}

function getSwitchItems(container: HTMLElement, mode: MagneticMode): HTMLElement[] {
  const candidates =
    mode === "rail"
      ? Array.from(
          container.querySelectorAll<HTMLElement>(
            "button.nav-item:not(:disabled):not([data-magnetic-ignore]), button[data-magnetic-item]:not(:disabled)"
          )
        )
      : Array.from(
          container.querySelectorAll<HTMLElement>(
            ':scope > button:not(:disabled):not([data-magnetic-ignore]), [role="tab"]:not(:disabled):not([data-magnetic-ignore])'
          )
        );
  return candidates.filter((btn) => btn.closest(ANY_GROUP_SELECTOR) === container);
}

function findActiveItem(items: HTMLElement[]): HTMLElement | null {
  return (
    items.find(
      (btn) =>
        btn.classList.contains("active") ||
        btn.getAttribute("aria-selected") === "true" ||
        btn.getAttribute("aria-current") === "page"
    ) ?? null
  );
}

function createOverlayEl(className: string): HTMLDivElement {
  const el = document.createElement("div");
  el.className = className;
  el.setAttribute("aria-hidden", "true");
  return el;
}

function measureInContainer(container: HTMLElement, item: HTMLElement): RelativeBox {
  const offsetBox = measureOffsetBox(
    container as unknown as OffsetNodeLike,
    item as unknown as OffsetNodeLike
  );
  if (offsetBox && offsetBox.w > 0 && offsetBox.h > 0) {
    return offsetBox;
  }
  return measureItemBox(
    container.getBoundingClientRect(),
    item.getBoundingClientRect(),
    container.scrollLeft,
    container.scrollTop,
    container.clientLeft,
    container.clientTop
  );
}

function applyBoxStyle(el: HTMLElement, box: RelativeBox): void {
  el.style.transform = `translate3d(${box.x}px, ${box.y}px, 0)`;
  el.style.width = `${box.w}px`;
  el.style.height = `${box.h}px`;
}

function boxesEqual(a: RelativeBox | null, b: RelativeBox): boolean {
  if (!a) return false;
  return (
    Math.abs(a.x - b.x) < 0.5 &&
    Math.abs(a.y - b.y) < 0.5 &&
    Math.abs(a.w - b.w) < 0.5 &&
    Math.abs(a.h - b.h) < 0.5
  );
}

function cancelActiveAnimations(rec: ContainerRecord): void {
  if (rec.cometAnim) {
    rec.cometAnim.cancel();
    rec.cometAnim = null;
  }
  if (rec.rafId !== null) {
    window.cancelAnimationFrame(rec.rafId);
    rec.rafId = null;
  }
}

function adaptiveRailHeight(itemHeight: number): number {
  return Math.min(16, Math.max(12, Math.round(itemHeight * 0.55)));
}

function animateButtonIcon(btn: HTMLElement): void {
  const svg = btn.querySelector("svg");
  if (!svg || typeof svg.animate !== "function") return;
  svg.animate(
    [
      { transform: "scale(0.86)" },
      { transform: "scale(1.1)" },
      { transform: "scale(1)" },
    ],
    { duration: 200, easing: COMET_EASING }
  );
}

function syncContainer(container: HTMLElement): void {
  const rec = records.get(container);
  if (!rec) return;

  const items = getSwitchItems(container, rec.mode);
  const activeBtn = findActiveItem(items);

  if (!activeBtn) {
    cancelActiveAnimations(rec);
    rec.activeEl.style.opacity = "0";
    if (rec.wormEl) rec.wormEl.style.opacity = "0";
    rec.prevActiveBtn = null;
    rec.prevActiveBox = null;
    return;
  }

  const nextBox = measureInContainer(container, activeBtn);
  if (nextBox.w <= 0 || nextBox.h <= 0) {
    rec.activeEl.style.opacity = "0";
    if (rec.wormEl) rec.wormEl.style.opacity = "0";
    return;
  }

  const reduced = prefersReducedMotion();
  const changedBtn = rec.initialized && rec.prevActiveBtn !== activeBtn;

  if (rec.initialized && !changedBtn && boxesEqual(rec.prevActiveBox, nextBox)) {
    return;
  }

  const railHeight = adaptiveRailHeight(nextBox.h);

  if (!rec.initialized || reduced) {
    cancelActiveAnimations(rec);
    rec.activeEl.style.transition = "none";
    applyBoxStyle(rec.activeEl, nextBox);
    rec.activeEl.style.opacity = "1";

    if (rec.wormEl) {
      const plan = computeCometPlan(null, nextBox, railHeight, 2, 34);
      rec.wormEl.style.transform = `translate3d(${plan.finalLeft}px, ${plan.finalTop}px, 0)`;
      rec.wormEl.style.height = `${plan.finalHeight}px`;
      rec.wormEl.style.opacity = "1";
    }

    if (!reduced) {
      rec.rafId = window.requestAnimationFrame(() => {
        rec.rafId = null;
        rec.activeEl.style.transition = "";
      });
    }
  } else {
    rec.activeEl.style.transition = "";
    applyBoxStyle(rec.activeEl, nextBox);
    rec.activeEl.style.opacity = "1";

    if (changedBtn) {
      animateButtonIcon(activeBtn);
    }

    if (rec.wormEl) {
      if (rec.cometAnim) {
        rec.cometAnim.cancel();
        rec.cometAnim = null;
      }

      const plan = computeCometPlan(
        changedBtn ? rec.prevActiveBox : null,
        nextBox,
        railHeight,
        2,
        34
      );

      // 始终先写入最终静止几何，保证动画结束或降级时落位零偏差
      rec.wormEl.style.transform = `translate3d(${plan.finalLeft}px, ${plan.finalTop}px, 0)`;
      rec.wormEl.style.height = `${plan.finalHeight}px`;
      rec.wormEl.style.opacity = "1";

      if (
        changedBtn &&
        plan.kind !== "static" &&
        typeof rec.wormEl.animate === "function"
      ) {
        const anim = rec.wormEl.animate(
          [
            {
              transform: `translate3d(${plan.oldLeft}px, ${plan.oldTop}px, 0)`,
              height: `${plan.oldHeight}px`,
            },
            {
              transform: `translate3d(${plan.midLeft}px, ${plan.midTop}px, 0)`,
              height: `${plan.midHeight}px`,
              offset: plan.midOffset,
            },
            {
              transform: `translate3d(${plan.finalLeft}px, ${plan.finalTop}px, 0)`,
              height: `${plan.finalHeight}px`,
            },
          ],
          {
            duration: COMET_DURATION_MS,
            easing: COMET_EASING,
          }
        );
        rec.cometAnim = anim;
        anim.onfinish = () => {
          if (rec.cometAnim === anim) rec.cometAnim = null;
        };
        anim.oncancel = () => {
          if (rec.cometAnim === anim) rec.cometAnim = null;
        };
      }
    }
  }

  rec.initialized = true;
  rec.prevActiveBtn = activeBtn;
  rec.prevActiveBox = nextBox;
}

function attachContainer(container: HTMLElement): void {
  if (records.has(container) || isNestedInsideManagedGroup(container)) return;
  const mode = resolveMode(container);
  if (!mode) return;

  container.setAttribute("data-ms-ready", mode);

  const hoverEl = createOverlayEl("ms-hover-pill");
  const activeEl = createOverlayEl("ms-active-pill");
  const wormEl = mode === "rail" ? createOverlayEl("ms-worm-rail") : null;

  container.append(hoverEl, activeEl);
  if (wormEl) container.append(wormEl);

  const onPointerOver = (event: PointerEvent) => {
    const target = (event.target as HTMLElement | null)?.closest<HTMLElement>(
      "button"
    );
    if (!target || !container.contains(target)) return;
    const items = getSwitchItems(container, mode);
    if (!items.includes(target)) return;

    const activeBtn = findActiveItem(items);
    if (target === activeBtn) {
      hoverEl.style.opacity = "0";
      return;
    }
    const box = measureInContainer(container, target);
    if (box.w <= 0 || box.h <= 0) return;
    applyBoxStyle(hoverEl, box);
    hoverEl.style.opacity = "1";
  };

  const onPointerLeave = () => {
    hoverEl.style.opacity = "0";
  };

  container.addEventListener("pointerover", onPointerOver);
  container.addEventListener("pointerleave", onPointerLeave);

  let scheduled = false;
  const scheduleSync = () => {
    if (scheduled) return;
    scheduled = true;
    queueMicrotask(() => {
      scheduled = false;
      if (container.isConnected) {
        syncContainer(container);
      }
    });
  };

  const rec: ContainerRecord = {
    mode,
    hoverEl,
    activeEl,
    wormEl,
    prevActiveBtn: null,
    prevActiveBox: null,
    initialized: false,
    cometAnim: null,
    rafId: null,
    mutObserver: null as unknown as MutationObserver,
    resizeObserver: null as unknown as ResizeObserver,
    cleanupListeners: () => {
      container.removeEventListener("pointerover", onPointerOver);
      container.removeEventListener("pointerleave", onPointerLeave);
    },
  };

  const mutObserver = new MutationObserver((mutations) => {
    let hasStructuralChange = false;
    let hasRelevantAttr = false;

    for (const m of mutations) {
      if (m.target === hoverEl || m.target === activeEl || m.target === wormEl) {
        continue;
      }
      if (m.type === "childList") {
        const external = Array.from(m.addedNodes)
          .concat(Array.from(m.removedNodes))
          .some((n) => n !== hoverEl && n !== activeEl && n !== wormEl);
        if (external) hasStructuralChange = true;
      } else if (m.type === "attributes") {
        hasRelevantAttr = true;
      }
    }

    if (!hasStructuralChange && !hasRelevantAttr) return;

    if (!hasStructuralChange && hasRelevantAttr) {
      const items = getSwitchItems(container, mode);
      const currentActive = findActiveItem(items);
      if (currentActive === rec.prevActiveBtn) {
        return;
      }
    }

    scheduleSync();
  });

  mutObserver.observe(container, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["class", "aria-selected", "aria-current", "disabled"],
  });

  const resizeObserver = new ResizeObserver(() => {
    if (container.isConnected) {
      syncContainer(container);
    }
  });
  resizeObserver.observe(container);

  rec.mutObserver = mutObserver;
  rec.resizeObserver = resizeObserver;

  records.set(container, rec);
  activeContainers.add(container);
  syncContainer(container);
}

function detachContainer(container: HTMLElement): void {
  const rec = records.get(container);
  if (!rec) return;
  cancelActiveAnimations(rec);
  rec.mutObserver.disconnect();
  rec.resizeObserver.disconnect();
  rec.cleanupListeners();
  rec.hoverEl.remove();
  rec.activeEl.remove();
  rec.wormEl?.remove();
  container.removeAttribute("data-ms-ready");
  records.delete(container);
  activeContainers.delete(container);
}

function scanTree(root: ParentNode): void {
  if (root instanceof HTMLElement && root.matches(ANY_GROUP_SELECTOR)) {
    attachContainer(root);
  }
  root.querySelectorAll<HTMLElement>(ANY_GROUP_SELECTOR).forEach(attachContainer);
}

/**
 * 安装全局「改版 B · 流光水滴短轨」按钮切换动效引擎：
 * - 左侧导航（rail）启用：悬停预瞄 + 2D 滑行背板 + 上限 34px 连续水滴短轨（跨列 6px 收短斜滑）+ 图标微弹
 * - 分段控制 / 页签（pill）启用：悬停预瞄 + 2D 滑行背板 + 图标微弹（无左蓝轨）
 * - 自动监听 DOM 动态挂载，未来新增同类切换按钮自动继承。
 */
export function installGlobalMagneticSwitch(): () => void {
  if (typeof document === "undefined") return () => {};
  if (globalInstalledCleanup) return globalInstalledCleanup;

  scanTree(document);

  const docObserver = new MutationObserver((mutations) => {
    for (const m of mutations) {
      for (const node of m.addedNodes) {
        if (node instanceof HTMLElement) {
          scanTree(node);
        }
      }
      for (const node of m.removedNodes) {
        if (node instanceof HTMLElement) {
          for (const managed of Array.from(activeContainers)) {
            if (!managed.isConnected || node === managed || node.contains(managed)) {
              detachContainer(managed);
            }
          }
        }
      }
    }
  });

  docObserver.observe(document.body, { childList: true, subtree: true });

  const cleanup = () => {
    docObserver.disconnect();
    for (const container of Array.from(activeContainers)) {
      detachContainer(container);
    }
    globalInstalledCleanup = null;
  };
  globalInstalledCleanup = cleanup;
  return cleanup;
}
