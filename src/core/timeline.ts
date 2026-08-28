import { clamp } from "./num";

/** A half-open span of time in seconds, used for both zoom and clip extents. */
export interface TimeWindow {
  readonly start: number;
  readonly end: number;
}

/** Deepest zoom, in seconds across the whole track — under one frame at 60fps. */
export const MIN_SPAN = 0.02;

export const span = (w: TimeWindow): number => w.end - w.start;

const usable = (w: TimeWindow): boolean =>
  Number.isFinite(w.start) && Number.isFinite(w.end) && w.end > w.start;

/**
 * Force `w` inside `bounds`, preserving its span where possible.
 *
 * A window that is too wide collapses onto `bounds`; one that runs off an edge
 * slides back in rather than being squashed, so zooming near either end keeps
 * the zoom level the user chose. A degenerate window (empty, inverted or
 * non-finite) resets to `bounds` — recoverable, and obvious on screen.
 */
export function clampWindow(
  w: TimeWindow,
  bounds: TimeWindow,
  minSpan: number = MIN_SPAN,
): TimeWindow {
  if (!usable(bounds)) return { start: bounds.start, end: bounds.start };
  if (!usable(w)) return bounds;

  const limit = span(bounds);
  const want = clamp(span(w), Math.min(minSpan, limit), limit);
  const start = clamp(w.start, bounds.start, bounds.end - want);
  return { start, end: start + want };
}

/**
 * Scale the window by `factor` while holding `focus` at the same on-screen
 * position — so wheel-zoom keeps the frame under the cursor put.
 */
export function zoomAround(
  w: TimeWindow,
  focus: number,
  factor: number,
  bounds: TimeWindow,
  minSpan: number = MIN_SPAN,
): TimeWindow {
  if (!usable(w) || !usable(bounds)) return clampWindow(w, bounds, minSpan);
  if (!Number.isFinite(factor) || factor <= 0) return w;

  const limit = span(bounds);
  const next = clamp(span(w) * factor, Math.min(minSpan, limit), limit);
  const anchor = clamp(focus, w.start, w.end);
  const fraction = (anchor - w.start) / span(w);
  const start = anchor - fraction * next;

  return clampWindow({ start, end: start + next }, bounds, minSpan);
}

export const panBy = (
  w: TimeWindow,
  deltaSeconds: number,
  bounds: TimeWindow,
): TimeWindow =>
  Number.isFinite(deltaSeconds)
    ? clampWindow({ start: w.start + deltaSeconds, end: w.end + deltaSeconds }, bounds)
    : w;

export function centerOn(
  w: TimeWindow,
  t: number,
  bounds: TimeWindow,
): TimeWindow {
  if (!Number.isFinite(t)) return w;
  const half = span(w) / 2;
  return clampWindow({ start: t - half, end: t + half }, bounds);
}

export const timeToFraction = (t: number, w: TimeWindow): number =>
  span(w) > 0 ? (t - w.start) / span(w) : 0;

export const fractionToTime = (f: number, w: TimeWindow): number =>
  w.start + f * span(w);

export const timeToX = (t: number, w: TimeWindow, width: number): number =>
  timeToFraction(t, w) * width;

export const xToTime = (x: number, w: TimeWindow, width: number): number =>
  width > 0 ? fractionToTime(x / width, w) : w.start;

/** Ruler steps that read naturally as time, rather than arbitrary decimals. */
const TICK_STEPS = [
  0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60,
  120, 300, 600, 900, 1800, 3600, 7200, 21600, 43200, 86400,
] as const;

const FINEST_TICK = TICK_STEPS[0];
const COARSEST_TICK = TICK_STEPS.at(-1) ?? FINEST_TICK;

/** The coarsest step that still gives roughly one label per `targetPx`. */
export function chooseTickStep(
  spanSeconds: number,
  widthPx: number,
  targetPx = 90,
): number {
  if (!(spanSeconds > 0) || !(widthPx > 0)) return FINEST_TICK;
  const minimum = (targetPx / widthPx) * spanSeconds;
  return TICK_STEPS.find((s) => s >= minimum) ?? COARSEST_TICK;
}

/** Guard against pathological tick counts when a window is briefly degenerate. */
const MAX_TICKS = 4096;

export function generateTicks(w: TimeWindow, step: number): readonly number[] {
  if (!(step > 0) || !usable(w)) return [];
  const first = Math.ceil(w.start / step);
  const last = Math.floor(w.end / step);
  const count = last - first + 1;
  if (count <= 0 || count > MAX_TICKS) return [];
  // Multiply the index rather than accumulating, so ticks never drift.
  return Array.from({ length: count }, (_, i) => (first + i) * step);
}
