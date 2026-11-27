export const MODAL_OPEN_MS = 220;
export const MODAL_BACKDROP_OPEN_MS = 190;
export const MODAL_CLOSE_MS = 165;
export const MODAL_OPEN_SCALE = 0.88;
export const MODAL_CLOSE_SCALE = 0.9;
export const MODAL_ORIGIN_FRESH_MS = 650;
export const MODAL_FALLBACK_OFFSET_Y = 12;

const MODAL_ORIGIN_FACTOR = 0.38;
const MODAL_ORIGIN_MAX_X = 220;
const MODAL_ORIGIN_MAX_Y = 140;

export interface OriginPoint {
  x: number;
  y: number;
}

export interface ViewportSize {
  width: number;
  height: number;
}

export interface ModalOriginVector {
  ox: number;
  oy: number;
  isPointerOrigin: boolean;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** Check whether a recorded trigger coordinate is still recent enough for the modal open event. */
export function isOriginFresh(
  recordedAt: number | null,
  now: number,
  maxAgeMs = MODAL_ORIGIN_FRESH_MS
): boolean {
  if (recordedAt === null || !Number.isFinite(recordedAt) || !Number.isFinite(now)) {
    return false;
  }
  const delta = now - recordedAt;
  return delta >= 0 && delta <= maxAgeMs;
}

/**
 * Compute clamped pixel origin offset relative to viewport center for Origin Morph Pop.
 * Falls back to subtle centered lift when no fresh trigger point exists (e.g. shortcuts / IPC).
 */
export function computeModalOriginVector(
  point: OriginPoint | null,
  viewport: ViewportSize
): ModalOriginVector {
  if (
    !point ||
    !Number.isFinite(point.x) ||
    !Number.isFinite(point.y) ||
    !Number.isFinite(viewport.width) ||
    !Number.isFinite(viewport.height) ||
    viewport.width <= 0 ||
    viewport.height <= 0
  ) {
    return { ox: 0, oy: MODAL_FALLBACK_OFFSET_Y, isPointerOrigin: false };
  }

  const centerX = viewport.width / 2;
  const centerY = viewport.height / 2;
  const rawX = Math.round((point.x - centerX) * MODAL_ORIGIN_FACTOR);
  const rawY = Math.round((point.y - centerY) * MODAL_ORIGIN_FACTOR);

  return {
    ox: clamp(rawX, -MODAL_ORIGIN_MAX_X, MODAL_ORIGIN_MAX_X),
    oy: clamp(rawY, -MODAL_ORIGIN_MAX_Y, MODAL_ORIGIN_MAX_Y),
    isPointerOrigin: true,
  };
}
