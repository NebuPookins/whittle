import { describe, expect, it } from "vitest";

import {
  describeFailure,
  errInfo,
  failureFromCode,
  failureFromPlayRejection,
  type MediaFailure,
} from "./media";

const ALL: readonly MediaFailure[] = [
  { kind: "aborted" },
  { kind: "network" },
  { kind: "decode" },
  { kind: "unreadable" },
  { kind: "stalled" },
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

describe("errInfo", () => {
  it("takes name and message from an Error", () => {
    const err = new Error("boom");
    err.name = "TypeError";
    expect(errInfo(err)).toEqual({ name: "TypeError", detail: "boom" });
  });

  it("labels a non-Error rejection as unknown rather than inventing a name", () => {
    expect(errInfo("nope")).toEqual({ name: "unknown", detail: "nope" });
  });

  it("reads a DOMException's name and message without needing it to be an Error", () => {
    // A DOMException is not an Error subclass in every engine (WebKit), so
    // errInfo must not depend on `instanceof Error` to reach these fields.
    const abort = new DOMException("The operation was aborted.", "AbortError");
    expect(errInfo(abort)).toEqual({
      name: "AbortError",
      detail: "The operation was aborted.",
    });
  });
});

describe("failureFromPlayRejection", () => {
  const abort = new Error("The operation was aborted.");
  abort.name = "AbortError";

  it("ignores AbortError, which a pause or seek causes, not a refusal", () => {
    expect(failureFromPlayRejection(abort)).toBeNull();
  });

  it("suppresses an AbortError DOMException, which is what play() rejects with", () => {
    const abort = new DOMException("The operation was aborted.", "AbortError");
    expect(failureFromPlayRejection(abort)).toBeNull();
  });

  it("reports any other rejection as a refusal", () => {
    const blocked = new Error("play() was blocked");
    blocked.name = "NotAllowedError";
    expect(failureFromPlayRejection(blocked)).toEqual({
      kind: "refused",
      detail: "play() was blocked",
    });
  });

  it("reports a non-Error rejection too", () => {
    expect(failureFromPlayRejection("nope")).toEqual({ kind: "refused", detail: "nope" });
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
    for (const failure of [{ kind: "decode" }, { kind: "unreadable" }, { kind: "stalled" }] as const) {
      expect(describeFailure(failure)).toMatch(/trimming still works/i);
    }
  });
});
