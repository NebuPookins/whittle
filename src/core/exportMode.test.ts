import { describe, expect, it } from "vitest";

import { exportModeFor } from "./exportMode";

describe("exportModeFor", () => {
  it("is frame-accurate only when wanted and supported", () => {
    const supported = { status: "supported" } as const;
    expect(exportModeFor(true, supported)).toBe("frameAccurate");
    expect(exportModeFor(false, supported)).toBe("keyframeCopy");
  });

  it("falls back to a keyframe copy for a file the backend cannot cut that way", () => {
    const unsupported = { status: "unsupported", reason: "not H.264" } as const;
    expect(exportModeFor(true, unsupported)).toBe("keyframeCopy");
  });
});
