import { describe, expect, it } from "vitest";

import { adjacentPlaybackRate } from "./playbackRate";

describe("adjacentPlaybackRate", () => {
  it("moves through the selectable speeds one option at a time", () => {
    expect(adjacentPlaybackRate(1, "slower")).toBe(0.5);
    expect(adjacentPlaybackRate(1, "faster")).toBe(1.25);
  });

  it("does not go past either end of the selectable speeds", () => {
    expect(adjacentPlaybackRate(0.25, "slower")).toBeNull();
    expect(adjacentPlaybackRate(4, "faster")).toBeNull();
  });
});
