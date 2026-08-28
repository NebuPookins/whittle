import { describe, expect, it } from "vitest";
import { editorReducer, initEditor, type EditorAction, type EditorState } from "./editor";
import { span } from "./timeline";

const run = (state: EditorState, ...actions: readonly EditorAction[]): EditorState =>
  actions.reduce(editorReducer, state);

const fresh = () => initEditor(100);

/** The invariants the reducer exists to guarantee. */
const expectConsistent = (s: EditorState) => {
  const { start, end } = s.history.clip;
  expect(s.view.start).toBeGreaterThanOrEqual(start - 1e-9);
  expect(s.view.end).toBeLessThanOrEqual(end + 1e-9);
  expect(s.playhead).toBeGreaterThanOrEqual(start - 1e-9);
  expect(s.playhead).toBeLessThanOrEqual(end + 1e-9);
  for (const m of [s.marks.inPoint, s.marks.outPoint]) {
    if (m !== null) {
      expect(m).toBeGreaterThanOrEqual(start - 1e-9);
      expect(m).toBeLessThanOrEqual(end + 1e-9);
    }
  }
};

describe("initEditor", () => {
  it("opens on the whole file", () => {
    const s = fresh();
    expect(s.history.clip).toEqual({ start: 0, end: 100 });
    expect(s.view).toEqual({ start: 0, end: 100 });
    expect(s.playhead).toBe(0);
    expect(s.marks).toEqual({ inPoint: null, outPoint: null });
  });
});

describe("seeking and marking", () => {
  it("moves the playhead", () => {
    expect(run(fresh(), { type: "seek", time: 42 }).playhead).toBe(42);
  });

  it("keeps the playhead and marks inside the clip", () => {
    const s = run(fresh(), { type: "seek", time: 999 }, { type: "markIn", time: -5 });
    expect(s.playhead).toBe(100);
    expect(s.marks.inPoint).toBe(0);
    expectConsistent(s);
  });

  it("keeps marks independent of one another", () => {
    const s = run(fresh(), { type: "markIn", time: 10 }, { type: "markOut", time: 90 });
    expect(s.marks).toEqual({ inPoint: 10, outPoint: 90 });
  });

  it("clears marks on request", () => {
    const s = run(fresh(), { type: "markIn", time: 10 }, { type: "clearMarks" });
    expect(s.marks).toEqual({ inPoint: null, outPoint: null });
  });

  it("does not disturb marks when seeking", () => {
    const s = run(fresh(), { type: "markIn", time: 10 }, { type: "seek", time: 50 });
    expect(s.marks.inPoint).toBe(10);
  });
});

describe("commit", () => {
  it("narrows the clip to the marked range and resets the view to it", () => {
    const s = run(
      fresh(),
      { type: "markIn", time: 20 },
      { type: "markOut", time: 30 },
      { type: "commit" },
    );
    expect(s.history.clip).toEqual({ start: 20, end: 30 });
    expect(s.view).toEqual({ start: 20, end: 30 });
    expect(s.marks).toEqual({ inPoint: null, outPoint: null });
    expectConsistent(s);
  });

  it("pulls an outside playhead into the new clip", () => {
    const s = run(
      fresh(),
      { type: "seek", time: 5 },
      { type: "markIn", time: 20 },
      { type: "markOut", time: 30 },
      { type: "commit" },
    );
    expect(s.playhead).toBe(20);
  });

  it("commits a single-sided discard", () => {
    const s = run(fresh(), { type: "markIn", time: 20 }, { type: "commit" });
    expect(s.history.clip).toEqual({ start: 20, end: 100 });
  });

  it("does nothing without marks", () => {
    const s = fresh();
    const after = run(s, { type: "commit" });
    expect(after.history).toBe(s.history);
  });

  it("narrows repeatedly, each round starting from the last", () => {
    const s = run(
      fresh(),
      { type: "markIn", time: 10 }, { type: "markOut", time: 90 }, { type: "commit" },
      { type: "markIn", time: 40 }, { type: "markOut", time: 60 }, { type: "commit" },
      { type: "markIn", time: 45 }, { type: "markOut", time: 50 }, { type: "commit" },
    );
    expect(s.history.clip).toEqual({ start: 45, end: 50 });
    expect(s.history.past).toHaveLength(3);
    expectConsistent(s);
  });
});

describe("undo and redo", () => {
  const narrowed = () =>
    run(
      fresh(),
      { type: "markIn", time: 20 },
      { type: "markOut", time: 30 },
      { type: "commit" },
    );

  it("restores the previous clip and its view", () => {
    const s = run(narrowed(), { type: "undo" });
    expect(s.history.clip).toEqual({ start: 0, end: 100 });
    expect(s.view).toEqual({ start: 0, end: 100 });
  });

  it("round-trips", () => {
    const s = run(narrowed(), { type: "undo" }, { type: "redo" });
    expect(s.history.clip).toEqual({ start: 20, end: 30 });
    expectConsistent(s);
  });

  it("discards marks that referred to the old clip", () => {
    const s = run(narrowed(), { type: "markIn", time: 25 }, { type: "undo" });
    expect(s.marks).toEqual({ inPoint: null, outPoint: null });
  });

  it("is inert at the ends of the history", () => {
    const s = fresh();
    expect(run(s, { type: "undo" })).toBe(s);
    expect(run(s, { type: "redo" })).toBe(s);
  });
});

describe("zoom and pan", () => {
  it("zooms in around a focus point", () => {
    const s = run(fresh(), { type: "zoom", factor: 0.5, focus: 50 });
    expect(span(s.view)).toBeCloseTo(50);
    expectConsistent(s);
  });

  it("never shows anything outside the clip", () => {
    const s = run(
      fresh(),
      { type: "markIn", time: 40 }, { type: "markOut", time: 60 }, { type: "commit" },
      { type: "pan", delta: -1000 },
    );
    expectConsistent(s);
    expect(s.view.start).toBeGreaterThanOrEqual(40);
  });

  it("fits the clip again", () => {
    const s = run(fresh(), { type: "zoom", factor: 0.1, focus: 20 }, { type: "zoomToFit" });
    expect(s.view).toEqual({ start: 0, end: 100 });
  });

  it("frames the pending selection with room to grab its edges", () => {
    const s = run(
      fresh(),
      { type: "markIn", time: 40 },
      { type: "markOut", time: 60 },
      { type: "zoomToSelection" },
    );
    expect(s.view.start).toBeLessThan(40);
    expect(s.view.end).toBeGreaterThan(60);
    expect(span(s.view)).toBeLessThan(30);
    expectConsistent(s);
  });

  it("leaves the clip untouched while zooming", () => {
    const s = run(fresh(), { type: "zoom", factor: 0.25, focus: 10 }, { type: "pan", delta: 3 });
    expect(s.history.clip).toEqual({ start: 0, end: 100 });
  });
});

describe("load", () => {
  it("replaces everything for the new file", () => {
    const s = run(
      fresh(),
      { type: "markIn", time: 20 },
      { type: "markOut", time: 30 },
      { type: "commit" },
      { type: "load", duration: 12 },
    );
    expect(s.history.clip).toEqual({ start: 0, end: 12 });
    expect(s.history.past).toHaveLength(0);
    expect(s.marks).toEqual({ inPoint: null, outPoint: null });
    expectConsistent(s);
  });
});
