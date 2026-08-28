import { describe, expect, it } from "vitest";

import { describeFailure, failureFromCode, type MediaFailure } from "./media";

const ALL: readonly MediaFailure[] = [
  { kind: "aborted" },
  { kind: "network" },
  { kind: "decode" },
  { kind: "unreadable" },
  { kind: "refused", detail: "NotAllowedError" },
];

describe("failureFromCode", () => {
  it("distinguishes the four codes the HTML spec defines", () => {
    const kinds = [1, 2, 3, 4].map((c) => failureFromCode(c).kind);
    expect(new Set(kinds).size).toBe(4);
  });

  it("treats a code outside the spec as unreadable rather than guessing", () => {
    for (const code of [0, 5, -1, 99, Number.NaN]) {
      expect(failureFromCode(code).kind).toBe("unreadable");
    }
  });
});

describe("describeFailure", () => {
  it("says something for every kind of failure", () => {
    for (const failure of ALL) {
      expect(describeFailure(failure).trim().length).toBeGreaterThan(0);
    }
  });

  it("includes the underlying reason when the player refused to start", () => {
    expect(describeFailure({ kind: "refused", detail: "NotAllowedError" })).toContain(
      "NotAllowedError",
    );
  });

  it("makes clear that a preview failure does not block trimming", () => {
    // The export path runs ffmpeg and never decodes, so a file the webview
    // cannot show is still perfectly cuttable. Saying otherwise would send
    // someone away from a file that works.
    for (const failure of [{ kind: "decode" }, { kind: "unreadable" }] as const) {
      expect(describeFailure(failure)).toMatch(/trimming still works/i);
    }
  });
});
