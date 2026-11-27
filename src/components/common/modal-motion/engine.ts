import {
  computeModalOriginVector,
  isOriginFresh,
  MODAL_BACKDROP_OPEN_MS,
  MODAL_CLOSE_MS,
  MODAL_CLOSE_SCALE,
  MODAL_OPEN_MS,
  MODAL_OPEN_SCALE,
  type OriginPoint,
} from "./geometry";

const TRIGGER_SELECTOR = "button, [role='menuitem'], [role='button'], a";
const GHOST_CLEANUP_BUFFER_MS = 60;

let installed = false;
let lastTrigger: (OriginPoint & { at: number }) | null = null;

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function getPanelElement(layer: HTMLElement): HTMLElement | null {
  return (
    layer.querySelector<HTMLElement>(
      ".dialog-material, .power-action-dialog, .dialog"
    ) ?? (layer.firstElementChild instanceof HTMLElement ? layer.firstElementChild : null)
  );
}

function collectModalLayers(node: Node): HTMLElement[] {
  if (!(node instanceof HTMLElement)) return [];
  if (node.dataset.modalGhost === "1") return [];
  const found: HTMLElement[] = [];
  if (node.classList.contains("modal-layer")) {
    found.push(node);
  }
  node.querySelectorAll<HTMLElement>(".modal-layer").forEach((el) => {
    if (el.dataset.modalGhost !== "1") {
      found.push(el);
    }
  });
  return found;
}

function syncClonedFormState(srcRoot: HTMLElement, dstRoot: HTMLElement): void {
  const srcInputs = srcRoot.querySelectorAll("input, textarea, select");
  const dstInputs = dstRoot.querySelectorAll("input, textarea, select");
  const len = Math.min(srcInputs.length, dstInputs.length);

  for (let i = 0; i < len; i++) {
    const src = srcInputs[i];
    const dst = dstInputs[i];
    if (src instanceof HTMLInputElement && dst instanceof HTMLInputElement) {
      dst.checked = src.checked;
      dst.value = src.value;
    } else if (src instanceof HTMLTextAreaElement && dst instanceof HTMLTextAreaElement) {
      dst.value = src.value;
    } else if (src instanceof HTMLSelectElement && dst instanceof HTMLSelectElement) {
      dst.selectedIndex = src.selectedIndex;
    }
  }
}

function sanitizeGhostTree(ghost: HTMLElement): void {
  ghost.classList.remove("modal-layer");
  ghost.classList.add("modal-exit-ghost");
  ghost.dataset.modalGhost = "1";
  ghost.setAttribute("aria-hidden", "true");
  ghost.inert = true;

  const walk = [ghost, ...Array.from(ghost.querySelectorAll<HTMLElement>("*"))];
  for (const el of walk) {
    el.removeAttribute("id");
    el.removeAttribute("role");
    el.removeAttribute("aria-label");
    el.removeAttribute("aria-labelledby");
    el.removeAttribute("data-ms-ready");
    if (el.hasAttribute("tabindex")) {
      el.setAttribute("tabindex", "-1");
    }
  }
}

function animateModalOpen(layer: HTMLElement): void {
  if (layer.dataset.modalAnimated === "1") return;
  layer.dataset.modalAnimated = "1";

  const now = performance.now();
  const originPoint =
    lastTrigger && isOriginFresh(lastTrigger.at, now)
      ? { x: lastTrigger.x, y: lastTrigger.y }
      : null;

  const { ox, oy } = computeModalOriginVector(originPoint, {
    width: window.innerWidth,
    height: window.innerHeight,
  });

  layer.dataset.modalOx = String(ox);
  layer.dataset.modalOy = String(oy);

  if (prefersReducedMotion()) return;

  if (typeof layer.animate === "function") {
    layer.animate([{ opacity: 0 }, { opacity: 1 }], {
      duration: MODAL_BACKDROP_OPEN_MS,
      easing: "ease-out",
    });
  }

  const panel = getPanelElement(layer);
  if (panel && typeof panel.animate === "function") {
    panel.animate(
      [
        {
          opacity: 0,
          transform: `translate3d(${ox}px, ${oy}px, 0) scale(${MODAL_OPEN_SCALE})`,
        },
        {
          opacity: 1,
          transform: "translate3d(0, 0, 0) scale(1)",
        },
      ],
      {
        duration: MODAL_OPEN_MS,
        easing: "cubic-bezier(0.22, 1, 0.36, 1)",
      }
    );
  }
}

function animateModalCloseGhost(layer: HTMLElement): void {
  if (layer.isConnected || layer.dataset.modalGhost === "1") return;
  if (prefersReducedMotion()) return;
  if (!document.body) return;

  const ox = Number(layer.dataset.modalOx ?? "0") || 0;
  const oy = Number(layer.dataset.modalOy ?? "12") || 0;

  const clone = layer.cloneNode(true);
  if (!(clone instanceof HTMLElement)) return;

  syncClonedFormState(layer, clone);
  sanitizeGhostTree(clone);
  document.body.appendChild(clone);

  let cleaned = false;
  const removeGhost = () => {
    if (cleaned) return;
    cleaned = true;
    clone.remove();
  };

  const fallbackTimer = window.setTimeout(
    removeGhost,
    MODAL_CLOSE_MS + GHOST_CLEANUP_BUFFER_MS
  );

  if (typeof clone.animate === "function") {
    const backdropAnim = clone.animate([{ opacity: 1 }, { opacity: 0 }], {
      duration: MODAL_CLOSE_MS,
      easing: "cubic-bezier(0.4, 0, 1, 1)",
      fill: "forwards",
    });
    backdropAnim.finished
      .catch(() => {})
      .finally(() => {
        window.clearTimeout(fallbackTimer);
        removeGhost();
      });
  }

  const panel = getPanelElement(clone);
  if (panel && typeof panel.animate === "function") {
    panel.animate(
      [
        {
          opacity: 1,
          transform: "translate3d(0, 0, 0) scale(1)",
        },
        {
          opacity: 0,
          transform: `translate3d(${ox}px, ${oy}px, 0) scale(${MODAL_CLOSE_SCALE})`,
        },
      ],
      {
        duration: MODAL_CLOSE_MS,
        easing: "cubic-bezier(0.4, 0, 1, 1)",
        fill: "forwards",
      }
    );
  }
}

function recordTriggerFromTarget(
  target: EventTarget | null,
  fallbackX?: number,
  fallbackY?: number
): void {
  const el = target instanceof Element ? target.closest<HTMLElement>(TRIGGER_SELECTOR) : null;
  if (el) {
    const rect = el.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      lastTrigger = {
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
        at: performance.now(),
      };
      return;
    }
  }
  if (typeof fallbackX === "number" && typeof fallbackY === "number") {
    lastTrigger = { x: fallbackX, y: fallbackY, at: performance.now() };
  }
}

function startObserving(): void {
  if (!document.body) return;
  document.querySelectorAll<HTMLElement>(".modal-layer").forEach(animateModalOpen);

  const observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      mutation.addedNodes.forEach((node) => {
        collectModalLayers(node).forEach(animateModalOpen);
      });
      mutation.removedNodes.forEach((node) => {
        collectModalLayers(node).forEach(animateModalCloseGhost);
      });
    }
  });

  observer.observe(document.body, { childList: true, subtree: true });
}

/** Install global Origin Morph Pop open/collapse animations for all existing and future modals. */
export function installGlobalModalMotion(): void {
  if (installed || typeof window === "undefined" || typeof document === "undefined") {
    return;
  }
  installed = true;

  window.addEventListener(
    "pointerdown",
    (event) => {
      recordTriggerFromTarget(event.target, event.clientX, event.clientY);
    },
    { capture: true, passive: true }
  );

  window.addEventListener(
    "keydown",
    (event) => {
      if (event.key === "Enter" || event.key === " ") {
        recordTriggerFromTarget(document.activeElement);
      }
    },
    { capture: true, passive: true }
  );

  if (document.body) {
    startObserving();
  } else {
    window.addEventListener("DOMContentLoaded", startObserving, { once: true });
  }
}
