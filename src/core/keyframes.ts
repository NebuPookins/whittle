import { at } from "./num";

/**
 * Keyframe lookups over a **sorted ascending** array of seconds.
 *
 * Cuts can only land on keyframes (see `trim::trim_args`), so these decide
 * where a trim will really begin.
 */

/** First index whose value is >= `t`, or `xs.length` if there is none. */
export function lowerBound(xs: readonly number[], t: number): number {
  const step = (lo: number, hi: number): number => {
    if (lo >= hi) return lo;
    const mid = (lo + hi) >>> 1;
    return at(xs, mid) < t ? step(mid + 1, hi) : step(lo, mid);
  };
  return step(0, xs.length);
}

/** First index whose value is > `t`, or `xs.length` if there is none. */
export function upperBound(xs: readonly number[], t: number): number {
  const step = (lo: number, hi: number): number => {
    if (lo >= hi) return lo;
    const mid = (lo + hi) >>> 1;
    return at(xs, mid) <= t ? step(mid + 1, hi) : step(lo, mid);
  };
  return step(0, xs.length);
}

export function keyframeAtOrBefore(
  xs: readonly number[],
  t: number,
): number | null {
  const i = upperBound(xs, t);
  return i > 0 ? at(xs, i - 1) : null;
}

export function keyframeAtOrAfter(
  xs: readonly number[],
  t: number,
): number | null {
  const i = lowerBound(xs, t);
  return i < xs.length ? at(xs, i) : null;
}

/**
 * The keyframe a cut at `t` would snap to.
 *
 * Ties go to the *earlier* keyframe: starting a cut earlier keeps footage,
 * starting it later loses footage the user asked to keep.
 */
export function nearestKeyframe(
  xs: readonly number[],
  t: number,
): number | null {
  const before = keyframeAtOrBefore(xs, t);
  const after = keyframeAtOrAfter(xs, t);
  if (before === null) return after;
  if (after === null) return before;
  return t - before <= after - t ? before : after;
}

/**
 * Merge two sorted keyframe lists, dropping duplicates.
 *
 * The scan streams in batches; because packets arrive in decode order a later
 * batch can start slightly before the previous one ended, so batches are
 * merged rather than appended — the array has to stay sorted for the binary
 * searches above to be correct.
 */
export function mergeSorted(
  a: readonly number[],
  b: readonly number[],
): readonly number[] {
  if (a.length === 0) return b;
  if (b.length === 0) return a;

  const out: number[] = [];
  const push = (v: number): void => {
    const last = out[out.length - 1];
    if (last === undefined || v - last > 1e-6) out.push(v);
  };

  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    const x = at(a, i);
    const y = at(b, j);
    if (x <= y) {
      push(x);
      i += 1;
    } else {
      push(y);
      j += 1;
    }
  }
  while (i < a.length) push(at(a, i++));
  while (j < b.length) push(at(b, j++));
  return out;
}

/** Keyframes within `[a, b]` inclusive — for drawing tick marks. */
export function keyframesInRange(
  xs: readonly number[],
  a: number,
  b: number,
): readonly number[] {
  if (b < a) return [];
  return xs.slice(lowerBound(xs, a), upperBound(xs, b));
}
