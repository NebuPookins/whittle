import { describe, expect, it } from "vitest";
import {
  chooseTickStep,
  clampWindow,
  centerOn,
  generateTicks,
  MIN_SPAN,
  panBy,
  span,
  timeToX,
  xToTime,
  zoomAround,
  type TimeWindow,
} from "./timeline";

const BOUNDS: TimeWindow = { start: 0, end: 100 };

describe("clampWindow", () => {
  it("leaves a window that already fits", () => {
    const w = { start: 10, end: 20 };
    expect(clampWindow(w, BOUNDS)).toEqual(w);
  });

  it("slides an overhanging window back in without shrinking it", () => {
    const w = clampWindow({ start: 95, end: 115 }, BOUNDS);
    expect(span(w)).toBeCloseTo(20);
    expect(w.end).toBeCloseTo(100);
    expect(w.start).toBeCloseTo(80);
  });

  it("collapses a window wider than the bounds onto the bounds", () => {
    expect(clampWindow({ start: -50, end: 500 }, BOUNDS)).toEqual(BOUNDS);
  });

  it("falls back to the whole track for a degenerate window", () => {
    // Recoverable and self-evident, rather than stranding the user zoomed
    // into an arbitrary 20ms of footage.
    expect(clampWindow({ start: 10, end: 10 }, BOUNDS)).toEqual(BOUNDS);
    expect(clampWindow({ start: 20, end: 10 }, BOUNDS)).toEqual(BOUNDS);
    expect(clampWindow({ start: Number.NaN, end: 10 }, BOUNDS)).toEqual(BOUNDS);
  });

  it("honours the minimum span when a real window is squeezed", () => {
    const w = clampWindow({ start: 10, end: 10 + MIN_SPAN / 4 }, BOUNDS);
    expect(span(w)).toBeCloseTo(MIN_SPAN);
  });

  it("survives a degenerate track", () => {
    const zero = { start: 0, end: 0 };
    expect(span(clampWindow({ start: 0, end: 5 }, zero))).toBe(0);
  });
});

describe("zoomAround", () => {
  it("holds the focus point at the same relative position", () => {
    const w = { start: 0, end: 100 };
    const focus = 25;
    const before = (focus - w.start) / span(w);
    const z = zoomAround(w, focus, 0.5, BOUNDS);
    expect((focus - z.start) / span(z)).toBeCloseTo(before, 6);
    expect(span(z)).toBeCloseTo(50);
  });

  it("zooms out symmetrically", () => {
    const z = zoomAround({ start: 40, end: 60 }, 50, 2, BOUNDS);
    expect(span(z)).toBeCloseTo(40);
    expect(z.start).toBeCloseTo(30);
  });

  it("stops at the deepest zoom", () => {
    const deep = Array.from({ length: 40 }).reduce<TimeWindow>(
      (w) => zoomAround(w, 50, 0.5, BOUNDS),
      BOUNDS,
    );
    expect(span(deep)).toBeCloseTo(MIN_SPAN);
  });

  it("never zooms out past the whole track", () => {
    const out = Array.from({ length: 40 }).reduce<TimeWindow>(
      (w) => zoomAround(w, 50, 2, BOUNDS),
      { start: 40, end: 60 },
    );
    expect(out).toEqual(BOUNDS);
  });

  it("keeps the zoom level when focusing on an edge", () => {
    const z = zoomAround({ start: 0, end: 100 }, 0, 0.25, BOUNDS);
    expect(span(z)).toBeCloseTo(25);
    expect(z.start).toBeCloseTo(0);
  });

  it("ignores a nonsensical factor", () => {
    const w = { start: 10, end: 20 };
    expect(zoomAround(w, 15, 0, BOUNDS)).toEqual(w);
    expect(zoomAround(w, 15, Number.NaN, BOUNDS)).toEqual(w);
  });
});

describe("panBy", () => {
  it("shifts the window", () => {
    expect(panBy({ start: 10, end: 20 }, 5, BOUNDS)).toEqual({ start: 15, end: 25 });
  });

  it("stops at the edges without changing the zoom level", () => {
    const w = panBy({ start: 90, end: 100 }, 50, BOUNDS);
    expect(w).toEqual({ start: 90, end: 100 });
    const back = panBy({ start: 0, end: 10 }, -50, BOUNDS);
    expect(back).toEqual({ start: 0, end: 10 });
  });
});

describe("centerOn", () => {
  it("puts the time in the middle when there is room", () => {
    const w = centerOn({ start: 0, end: 10 }, 50, BOUNDS);
    expect((w.start + w.end) / 2).toBeCloseTo(50);
  });

  it("clamps at the edges", () => {
    expect(centerOn({ start: 0, end: 10 }, 0, BOUNDS)).toEqual({ start: 0, end: 10 });
  });
});

describe("time <-> pixel mapping", () => {
  const w: TimeWindow = { start: 10, end: 20 };

  it("round-trips", () => {
    for (const t of [10, 12.5, 17, 20]) {
      expect(xToTime(timeToX(t, w, 800), w, 800)).toBeCloseTo(t, 9);
    }
  });

  it("maps the window onto the full width", () => {
    expect(timeToX(10, w, 800)).toBeCloseTo(0);
    expect(timeToX(20, w, 800)).toBeCloseTo(800);
    expect(timeToX(15, w, 800)).toBeCloseTo(400);
  });

  it("degrades safely at zero width", () => {
    expect(xToTime(0, w, 0)).toBe(10);
    expect(timeToX(15, { start: 5, end: 5 }, 800)).toBe(0);
  });
});

describe("chooseTickStep", () => {
  it("picks coarser steps as more time is on screen", () => {
    const narrow = chooseTickStep(1, 900);
    const wide = chooseTickStep(3600, 900);
    expect(narrow).toBeLessThan(wide);
  });

  it("keeps labels from crowding", () => {
    for (const s of [0.5, 5, 60, 900, 7200]) {
      const step = chooseTickStep(s, 900);
      expect((step / s) * 900).toBeGreaterThanOrEqual(90);
    }
  });

  it("returns a usable step for degenerate inputs", () => {
    expect(chooseTickStep(0, 900)).toBeGreaterThan(0);
    expect(chooseTickStep(10, 0)).toBeGreaterThan(0);
  });
});

describe("generateTicks", () => {
  it("emits aligned multiples of the step inside the window", () => {
    expect(generateTicks({ start: 0, end: 10 }, 2)).toEqual([0, 2, 4, 6, 8, 10]);
    expect(generateTicks({ start: 3, end: 9 }, 5)).toEqual([5]);
  });

  it("does not drift over many ticks", () => {
    const ticks = generateTicks({ start: 0, end: 100 }, 0.1);
    expect(ticks[ticks.length - 1]).toBeCloseTo(100, 9);
    expect(ticks[500]).toBeCloseTo(50, 9);
  });

  it("returns nothing for a step that cannot be drawn", () => {
    expect(generateTicks({ start: 0, end: 10 }, 0)).toEqual([]);
    expect(generateTicks({ start: 10, end: 10 }, 1)).toEqual([]);
    expect(generateTicks({ start: 0, end: 1e9 }, 0.001)).toEqual([]);
  });
});
