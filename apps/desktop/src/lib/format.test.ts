import { describe, expect, it } from "vitest";
import {
  formatBitrate,
  formatBytes,
  formatClock,
  formatDuration,
  formatSpan,
  parseClock,
} from "./format";

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

  it("formats rough durations for estimates", () => {
    expect(formatDuration(0.2)).toBe("1 s");
    expect(formatDuration(42.4)).toBe("42 s");
    expect(formatDuration(200)).toBe("3 min");
    expect(formatDuration(7200)).toBe("2 h");
    expect(formatDuration(Number.NaN)).toBe("–");
  });

  it("formats bitrates", () => {
    expect(formatBitrate(128_000)).toBe("128 kbps");
    expect(formatBitrate(1_411_200)).toBe("1.4 Mbps");
    expect(formatBitrate(3_840_000)).toBe("3.8 Mbps");
    expect(formatBitrate(12_400_000)).toBe("12 Mbps");
    expect(formatBitrate(999_700)).toBe("1 Mbps");
    expect(formatBitrate(847_000)).toBe("847 kbps");
    // Estimates don't pretend to be precise.
    expect(formatBitrate(847_000, true)).toBe("850 kbps");
    expect(formatBitrate(64_400, true)).toBe("64 kbps");
    expect(formatBitrate(0)).toBe("–");
  });
});
