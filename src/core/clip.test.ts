import { describe, expect, it } from "vitest";
import {
  canRedo,
  canUndo,
  commit,
  effectiveCut,
  hasPendingChange,
  initHistory,
  MIN_CLIP,
  NO_MARKS,
  pendingClip,
  redo,
  undo,
  type Clip,
} from "./clip";

const CLIP: Clip = { start: 10, end: 20 };
const KF = [0, 1.6, 3.2, 4.8, 6.4, 8.0, 9.6, 11.2];

describe("initHistory", () => {
  it("starts with the whole file and nothing to undo", () => {
    const h = initHistory(12);
    expect(h.clip).toEqual({ start: 0, end: 12 });
    expect(canUndo(h)).toBe(false);
    expect(canRedo(h)).toBe(false);
  });

  it("tolerates a file whose duration is unusable", () => {
    for (const bad of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(initHistory(bad).clip).toEqual({ start: 0, end: 0 });
    }
  });
});

describe("pendingClip", () => {
  it("applies each mark to its own edge", () => {
    expect(pendingClip(CLIP, { inPoint: 12, outPoint: null })).toEqual({ start: 12, end: 20 });
    expect(pendingClip(CLIP, { inPoint: null, outPoint: 18 })).toEqual({ start: 10, end: 18 });
    expect(pendingClip(CLIP, { inPoint: 12, outPoint: 18 })).toEqual({ start: 12, end: 18 });
  });

  it("is a no-op with no marks", () => {
    expect(pendingClip(CLIP, NO_MARKS)).toEqual(CLIP);
    expect(hasPendingChange(CLIP, NO_MARKS)).toBe(false);
  });

  it("clamps marks outside the current clip", () => {
    expect(pendingClip(CLIP, { inPoint: -5, outPoint: 999 })).toEqual(CLIP);
  });

  it("orders reversed marks rather than inverting the clip", () => {
    expect(pendingClip(CLIP, { inPoint: 18, outPoint: 12 })).toEqual({ start: 12, end: 18 });
  });

  it("refuses to narrow below the minimum clip length", () => {
    const tiny = pendingClip(CLIP, { inPoint: 15, outPoint: 15 + MIN_CLIP / 2 });
    expect(tiny).toEqual(CLIP);
  });
});

describe("commit / undo / redo", () => {
  it("records each narrowing step", () => {
    const h = commit(initHistory(100), { start: 10, end: 50 });
    expect(h.clip).toEqual({ start: 10, end: 50 });
    expect(canUndo(h)).toBe(true);
  });

  it("steps back and forward through the narrowing", () => {
    const a = initHistory(100);
    const b = commit(a, { start: 10, end: 50 });
    const c = commit(b, { start: 20, end: 30 });

    expect(undo(c).clip).toEqual(b.clip);
    expect(undo(undo(c)).clip).toEqual(a.clip);
    expect(redo(undo(c)).clip).toEqual(c.clip);
  });

  it("ignores a commit that changes nothing", () => {
    const h = initHistory(100);
    expect(commit(h, { start: 0, end: 100 })).toBe(h);
  });

  it("drops the redo branch once a new commit is made", () => {
    const c = commit(commit(initHistory(100), { start: 10, end: 50 }), { start: 20, end: 30 });
    const branched = commit(undo(c), { start: 15, end: 40 });
    expect(canRedo(branched)).toBe(false);
    expect(branched.clip).toEqual({ start: 15, end: 40 });
  });

  it("does nothing at either end of the history", () => {
    const h = initHistory(100);
    expect(undo(h)).toBe(h);
    expect(redo(h)).toBe(h);
  });

  it("survives a long undo/redo round trip", () => {
    const steps = [
      { start: 10, end: 90 },
      { start: 20, end: 80 },
      { start: 30, end: 70 },
      { start: 40, end: 60 },
    ];
    const forward = steps.reduce(commit, initHistory(100));
    const back = steps.reduce(undo, forward);
    expect(back.clip).toEqual({ start: 0, end: 100 });
    const again = steps.reduce(redo, back);
    expect(again.clip).toEqual({ start: 40, end: 60 });
  });
});

describe("effectiveCut", () => {
  it("snaps the start to the nearest keyframe and reports the drift", () => {
    const cut = effectiveCut({ start: 3.3, end: 9 }, KF);
    expect(cut.start).toBe(3.2);
    expect(cut.end).toBe(9);
    expect(cut.snapped).toBe(true);
    expect(cut.drift).toBeCloseTo(-0.1);
  });

  it("reports no snap when the start is already a keyframe", () => {
    const cut = effectiveCut({ start: 3.2, end: 9 }, KF);
    expect(cut.snapped).toBe(false);
    expect(cut.drift).toBe(0);
  });

  it("leaves the end alone — only the start must land on a keyframe", () => {
    expect(effectiveCut({ start: 0, end: 7.77 }, KF).end).toBe(7.77);
  });

  it("passes the clip through while keyframes are still unknown", () => {
    const cut = effectiveCut({ start: 3.3, end: 9 }, []);
    expect(cut.start).toBe(3.3);
    expect(cut.snapped).toBe(false);
  });

  it("falls back to an earlier keyframe rather than snapping past the end", () => {
    const cut = effectiveCut({ start: 11.15, end: 11.18 }, KF);
    expect(cut.start).toBe(9.6);
    expect(cut.start).toBeLessThan(cut.end);
    expect(cut.snapped).toBe(true);
    expect(cut.drift).toBeCloseTo(-1.55);
  });

  it("stays unsnapped when no keyframe precedes the in-point", () => {
    const cut = effectiveCut({ start: 1.0, end: 2.0 }, [5.0, 6.0]);
    expect(cut.start).toBe(1.0);
    expect(cut.snapped).toBe(false);
  });
});
