import { describe, expect, it } from "vitest";
import {
  canRedo,
  canUndo,
  commit,
  effectiveCut,
  hasPendingChange,
  initHistory,
  MIN_CLIP,
  nearestWaypoint,
  NO_MARKS,
  pendingClip,
  probePoint,
  redo,
  reencodeSpan,
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
  it("snaps the start back to the previous keyframe and reports the drift", () => {
    const cut = effectiveCut({ start: 3.3, end: 9 }, KF, "keyframeCopy");
    expect(cut.start).toBe(3.2);
    expect(cut.end).toBe(9);
    expect(cut.snapped).toBe(true);
    expect(cut.drift).toBeCloseTo(-0.1);
  });

  it("reports no snap when the start is already a keyframe", () => {
    const cut = effectiveCut({ start: 3.2, end: 9 }, KF, "keyframeCopy");
    expect(cut.snapped).toBe(false);
    expect(cut.drift).toBe(0);
  });

  it("leaves the end alone — only the start must land on a keyframe", () => {
    expect(effectiveCut({ start: 0, end: 7.77 }, KF, "keyframeCopy").end).toBe(7.77);
  });

  it("passes the clip through while keyframes are still unknown", () => {
    const cut = effectiveCut({ start: 3.3, end: 9 }, [], "keyframeCopy");
    expect(cut.start).toBe(3.3);
    expect(cut.snapped).toBe(false);
  });

  it("never snaps forward, even when the next keyframe is closer", () => {
    const cut = effectiveCut({ start: 4.7, end: 9 }, KF, "keyframeCopy");
    expect(cut.start).toBe(3.2);
    expect(cut.drift).toBeCloseTo(-1.5);
  });

  it("always starts on the latest keyframe at or before the in-point", () => {
    // Sweep in-points across every gap, including either side of each keyframe,
    // so a regression to "nearest" snapping fails wherever the next keyframe is
    // closer.
    const inPoints = Array.from({ length: 1201 }, (_, i) => i * 0.01);
    inPoints.forEach((inPoint) => {
      const cut = effectiveCut({ start: inPoint, end: 12 }, KF, "keyframeCopy");
      const expected = Math.max(...KF.filter((k) => k <= inPoint + 1e-9));
      expect(cut.start, `in-point ${inPoint}`).toBeCloseTo(expected, 9);
      expect(cut.start, `in-point ${inPoint}`).toBeLessThanOrEqual(inPoint + 1e-9);
    });
  });

  it("keeps a sub-GOP clip non-empty by snapping back", () => {
    const cut = effectiveCut({ start: 11.15, end: 11.18 }, KF, "keyframeCopy");
    expect(cut.start).toBe(9.6);
    expect(cut.start).toBeLessThan(cut.end);
    expect(cut.snapped).toBe(true);
  });

  it("stays unsnapped when no keyframe precedes the in-point", () => {
    const cut = effectiveCut({ start: 1.0, end: 2.0 }, [5.0, 6.0], "keyframeCopy");
    expect(cut.start).toBe(1.0);
    expect(cut.snapped).toBe(false);
  });
});

describe("nearestWaypoint", () => {
  it("falls back to the clip edges when nothing else is marked", () => {
    expect(nearestWaypoint(CLIP, NO_MARKS, [], 15, "back")).toBe(10);
    expect(nearestWaypoint(CLIP, NO_MARKS, [], 15, "forward")).toBe(20);
  });

  it("prefers the nearest mark over the clip edge", () => {
    const marks = { inPoint: 14, outPoint: 18 };
    expect(nearestWaypoint(CLIP, marks, [], 15, "back")).toBe(14);
    expect(nearestWaypoint(CLIP, marks, [], 15, "forward")).toBe(18);
  });

  it("hops between keep pins in either direction", () => {
    expect(nearestWaypoint(CLIP, NO_MARKS, [12, 15, 19], 15, "back")).toBe(12);
    expect(nearestWaypoint(CLIP, NO_MARKS, [12, 15, 19], 15, "forward")).toBe(19);
  });

  it("treats every waypoint alike regardless of its kind", () => {
    // A keep pin at 13 sits between the in-mark and the playhead, so it wins.
    const marks = { inPoint: 12, outPoint: 18 };
    expect(nearestWaypoint(CLIP, marks, [13, 17], 15, "back")).toBe(13);
    expect(nearestWaypoint(CLIP, marks, [13, 17], 15, "forward")).toBe(17);
  });

  it("ignores a waypoint sitting at the playhead", () => {
    // The in-mark coincides with the playhead, so the next one back is the edge.
    expect(nearestWaypoint(CLIP, { inPoint: 15, outPoint: null }, [], 15, "back")).toBe(10);
    expect(nearestWaypoint(CLIP, { inPoint: 15, outPoint: null }, [], 15, "forward")).toBe(20);
  });

  it("returns null at the edges when there is nowhere left to go", () => {
    expect(nearestWaypoint(CLIP, NO_MARKS, [15], 10, "back")).toBeNull();
    expect(nearestWaypoint(CLIP, NO_MARKS, [15], 20, "forward")).toBeNull();
  });
});

describe("effectiveCut in frame-accurate mode", () => {
  it("keeps the marked range without snapping", () => {
    expect(effectiveCut({ start: 3.3, end: 9 }, KF, "frameAccurate")).toEqual({
      start: 3.3,
      end: 9,
      drift: 0,
      snapped: false,
    });
  });

  it("needs no keyframes to know where it cuts", () => {
    const cut = effectiveCut({ start: 3.3, end: 9 }, [], "frameAccurate");
    expect(cut.start).toBe(3.3);
    expect(cut.snapped).toBe(false);
  });
});

describe("reencodeSpan", () => {
  it("counts the stretch before the first keyframe and after the last", () => {
    // Keyframes inside 3.3..9 run from 4.8 to 8.0: 1.5s before, 1.0s after.
    expect(reencodeSpan({ start: 3.3, end: 9 }, KF, 30)).toBeCloseTo(2.5);
  });

  it("is zero when both ends sit on keyframes", () => {
    expect(reencodeSpan({ start: 3.2, end: 8 }, KF, 30)).toBe(0);
  });

  it("counts an edge within half a frame of a keyframe as on it", () => {
    expect(reencodeSpan({ start: 3.2 + 0.01, end: 8 - 0.01 }, KF, 30)).toBe(0);
    // Without a frame rate there is no slack to apply.
    expect(reencodeSpan({ start: 3.21, end: 7.99 }, KF, null)).toBeCloseTo(
      4.8 - 3.21 + (7.99 - 6.4),
    );
  });

  it("re-encodes the whole clip when it holds fewer than two keyframes", () => {
    expect(reencodeSpan({ start: 3.3, end: 4.0 }, KF, 30)).toBeCloseTo(0.7);
    expect(reencodeSpan({ start: 3.3, end: 3.4 }, [], 30)).toBeCloseTo(0.1);
  });
});

describe("probePoint", () => {
  const SEL: Clip = { start: 0, end: 100 };

  it("bisects the whole selection when nothing is pinned", () => {
    expect(probePoint(SEL, [])).toBe(50);
  });

  it("bisects the front stretch when it is the longer end", () => {
    expect(probePoint(SEL, [70])).toBe(35);
  });

  it("bisects the back stretch when it is the longer end", () => {
    expect(probePoint(SEL, [30])).toBe(65);
    expect(probePoint(SEL, [10, 70])).toBe(85);
  });

  it("prefers the front on a tie", () => {
    expect(probePoint(SEL, [50])).toBe(25);
  });

  it("ignores pins outside or on the edges of the selection", () => {
    expect(probePoint({ start: 50, end: 100 }, [10, 50, 60, 100, 120])).toBe(80);
  });

  it("does not depend on the order of the pins", () => {
    expect(probePoint(SEL, [70, 10])).toBe(probePoint(SEL, [10, 70]));
  });
});
