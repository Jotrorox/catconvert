import { describe, expect, it } from "vitest";
import { formatBytes, formatClock, formatSpan, parseClock } from "./format";

describe("format", () => {
  it("formats clock times", () => {
    expect(formatClock(42)).toBe("0:42");
    expect(formatClock(3.24, true)).toBe("0:03.2");
    expect(formatClock(3725)).toBe("1:02:05");
    expect(formatClock(59.96, true)).toBe("1:00.0");
  });

  it("parses what it formats, and friendlier input", () => {
    expect(parseClock("0:03.2")).toBeCloseTo(3.2);
    expect(parseClock("42")).toBe(42);
    expect(parseClock("1:02:05")).toBe(3725);
    expect(parseClock(" 1:30 ")).toBe(90);
    expect(parseClock("1:x")).toBeNull();
    expect(parseClock("1.5:00")).toBeNull();
    expect(parseClock("")).toBeNull();
  });

  it("formats spans and sizes", () => {
    expect(formatSpan(27.84)).toBe("27.8 s");
    expect(formatSpan(245)).toBe("4 min 05 s");
    expect(formatBytes(48_000_000)).toBe("48.0 MB");
  });
});
