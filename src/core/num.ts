export const clamp = (v: number, lo: number, hi: number): number =>
  Math.min(Math.max(v, lo), hi);

/** Times are compared with a tolerance well below one video frame. */
export const EPSILON = 1e-9;

export const sameTime = (a: number, b: number): boolean =>
  Math.abs(a - b) < EPSILON;

/** Read an array slot, refusing to silently produce `undefined`. */
export function at(xs: readonly number[], i: number): number {
  const v = xs[i];
  if (v === undefined) throw new RangeError(`index ${i} is out of range`);
  return v;
}
