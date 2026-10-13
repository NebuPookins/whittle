import { describe, expect, it } from "vitest";
import {
  keyframeAtOrAfter,
  keyframeAtOrBefore,
  keyframesInRange,
  mergeSorted,
} from "./keyframes";

// The keyframe layout ffprobe reports for a 12s clip at GOP 48 / 30fps.
const KF = [0, 1.6, 3.2, 4.8, 6.4, 8.0, 9.6, 11.2];

describe("keyframeAtOrBefore / keyframeAtOrAfter", () => {
  it("include an exact hit", () => {
    expect(keyframeAtOrBefore(KF, 3.2)).toBe(3.2);
    expect(keyframeAtOrAfter(KF, 3.2)).toBe(3.2);
  });

  it("step to the neighbouring keyframe otherwise", () => {
    expect(keyframeAtOrBefore(KF, 3.3)).toBe(3.2);
    expect(keyframeAtOrAfter(KF, 3.3)).toBe(4.8);
  });

  it("return null when there is nothing on that side", () => {
    expect(keyframeAtOrBefore(KF, -1)).toBeNull();
    expect(keyframeAtOrAfter(KF, 12)).toBeNull();
    expect(keyframeAtOrBefore([], 1)).toBeNull();
    expect(keyframeAtOrAfter([], 1)).toBeNull();
  });
});

describe("keyframesInRange", () => {
  it("returns the keyframes inside the window, endpoints included", () => {
    expect(keyframesInRange(KF, 3.2, 6.4)).toEqual([3.2, 4.8, 6.4]);
  });

  it("returns an empty list for an empty or inverted window", () => {
    expect(keyframesInRange(KF, 3.3, 3.4)).toEqual([]);
    expect(keyframesInRange(KF, 6.4, 3.2)).toEqual([]);
    expect(keyframesInRange([], 0, 10)).toEqual([]);
  });

  it("covers the whole list when the window spans everything", () => {
    expect(keyframesInRange(KF, -5, 100)).toEqual(KF);
  });
});

describe("mergeSorted", () => {
  it("keeps the result sorted", () => {
    expect(mergeSorted([0, 3.2, 6.4], [1.6, 4.8])).toEqual([0, 1.6, 3.2, 4.8, 6.4]);
  });

  it("handles a batch that overlaps the previous one", () => {
    // Decode order means a later batch can start before the last one ended.
    expect(mergeSorted([0, 1.6, 4.8], [3.2, 6.4])).toEqual([0, 1.6, 3.2, 4.8, 6.4]);
  });

  it("drops duplicates across the seam", () => {
    expect(mergeSorted([0, 1.6, 3.2], [3.2, 4.8])).toEqual([0, 1.6, 3.2, 4.8]);
  });

  it("returns the other side when one is empty", () => {
    expect(mergeSorted([], [1, 2])).toEqual([1, 2]);
    expect(mergeSorted([1, 2], [])).toEqual([1, 2]);
    expect(mergeSorted([], [])).toEqual([]);
  });

  it("supports accumulating many batches in order", () => {
    const batches = [[0, 1.6], [3.2, 4.8], [6.4, 8.0], [9.6, 11.2]];
    const all = batches.reduce<readonly number[]>((acc, b) => mergeSorted(acc, b), []);
    expect(all).toEqual(KF);
    expect(keyframeAtOrAfter(all, 6.3)).toBe(6.4);
  });
});

describe("lookup over a large list", () => {
  const many = Array.from({ length: 100_000 }, (_, i) => i * 0.5);

  it("stays exact at arbitrary offsets", () => {
    expect(keyframeAtOrBefore(many, 25_000.26)).toBe(25_000);
    expect(keyframeAtOrAfter(many, 25_000.24)).toBe(25_000.5);
  });

  it("does not blow the stack", () => {
    expect(() => keyframeAtOrBefore(many, 1)).not.toThrow();
  });
});
