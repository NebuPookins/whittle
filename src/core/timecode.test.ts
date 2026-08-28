import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatSpan,
  formatTickLabel,
  formatTimecode,
  parseTimecode,
} from "./timecode";

describe("formatTimecode", () => {
  it("renders hours, minutes, seconds and milliseconds", () => {
    expect(formatTimecode(0)).toBe("00:00:00.000");
    expect(formatTimecode(3661.5)).toBe("01:01:01.500");
    expect(formatTimecode(59.999)).toBe("00:00:59.999");
  });

  it("carries rounding into the next second", () => {
    expect(formatTimecode(59.9999)).toBe("00:01:00.000");
  });

  it("clamps values that cannot be shown on a timeline", () => {
    expect(formatTimecode(-5)).toBe("00:00:00.000");
    expect(formatTimecode(Number.NaN)).toBe("00:00:00.000");
    expect(formatTimecode(Number.POSITIVE_INFINITY)).toBe("00:00:00.000");
  });
});

describe("parseTimecode", () => {
  it("reads the formats a user might type", () => {
    expect(parseTimecode("83.5")).toBe(83.5);
    expect(parseTimecode("1:23")).toBe(83);
    expect(parseTimecode("1:01:01.5")).toBe(3661.5);
    expect(parseTimecode("  2:00  ")).toBe(120);
    expect(parseTimecode(".5")).toBe(0.5);
  });

  it("round-trips what formatTimecode produces", () => {
    for (const t of [0, 1.25, 83.5, 3661.5, 7322.125]) {
      expect(parseTimecode(formatTimecode(t))).toBeCloseTo(t, 3);
    }
  });

  it("rejects input it cannot interpret", () => {
    for (const bad of ["", "abc", "1:2:3:4", "-5", "1:", ":", "1::2", "1e3"]) {
      expect(parseTimecode(bad)).toBeNull();
    }
  });
});

describe("formatTickLabel", () => {
  it("shows no decimals for coarse steps", () => {
    expect(formatTickLabel(750, 30)).toBe("12:30");
  });

  it("adds precision as the step shrinks", () => {
    expect(formatTickLabel(750.25, 0.1)).toBe("12:30.3");
    expect(formatTickLabel(750.25, 0.01)).toBe("12:30.25");
    expect(formatTickLabel(750.25, 0.001)).toBe("12:30.250");
  });

  it("includes hours only once past an hour", () => {
    expect(formatTickLabel(3723, 1)).toBe("1:02:03");
    expect(formatTickLabel(123, 1)).toBe("2:03");
  });

  it("never renders a sixtieth second", () => {
    expect(formatTickLabel(59.9999, 1)).toBe("1:00");
  });
});

describe("formatSpan", () => {
  it("keeps precision for the short spans that matter when zoomed in", () => {
    expect(formatSpan(4.2)).toBe("4.20s");
    expect(formatSpan(42.25)).toBe("42.3s");
  });

  it("switches to clock form for longer spans", () => {
    expect(formatSpan(83)).toBe("1:23");
    expect(formatSpan(3723)).toBe("1:02:03");
  });

  it("clamps nonsense to zero", () => {
    expect(formatSpan(-1)).toBe("0.00s");
    expect(formatSpan(Number.NaN)).toBe("0.00s");
  });
});

describe("formatBytes", () => {
  it("scales to a readable unit", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1.00 KB");
    expect(formatBytes(1024 * 1024 * 5)).toBe("5.00 MB");
    expect(formatBytes(1024 ** 3 * 12.5)).toBe("12.5 GB");
  });

  it("marks unknown sizes rather than inventing one", () => {
    expect(formatBytes(-1)).toBe("—");
    expect(formatBytes(Number.NaN)).toBe("—");
  });
});
