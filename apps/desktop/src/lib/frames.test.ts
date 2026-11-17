import { describe, expect, it } from "vitest";
import { frameIndex, frameTime, stepFrame } from "./frames";

/** Seconds as ffmpeg's `-ss` gets them (convert.rs `format_secs`). */
const asFfmpeg = (t: number) => Math.round(t * 1000) / 1000;

describe("frame stepping", () => {
  it("lands on each frame even after millisecond rounding", () => {
    for (const fps of [24, 29.97, 30, 60, 120, 240]) {
      for (let n = 0; n < 500; n++) {
        expect(frameIndex(asFfmpeg(frameTime(n, fps)), fps)).toBe(n);
      }
    }
  });

  it("reads the frame shown at any time as the next frame start", () => {
    expect(frameIndex(0, 30)).toBe(0);
    expect(frameIndex(1, 30)).toBe(30);
    expect(frameIndex(1.01, 30)).toBe(31);
    expect(frameIndex(1.05, 30)).toBe(32);
  });

  it("steps one frame at a time", () => {
    const next = stepFrame(1, 1, 30, 10);
    expect(frameIndex(next, 30)).toBe(31);
    expect(frameIndex(stepFrame(next, 1, 30, 10), 30)).toBe(32);
    expect(frameIndex(stepFrame(next, -1, 30, 10), 30)).toBe(30);
    // From between frames, back goes to the one before the frame on screen.
    expect(frameIndex(stepFrame(1.05, -1, 30, 10), 30)).toBe(31);
  });

  it("stays within the clip", () => {
    expect(stepFrame(0, -1, 30, 10)).toBe(0);
    const last = stepFrame(10, 1, 30, 10);
    expect(frameIndex(last, 30)).toBe(299);
    expect(last).toBeLessThan(10 - 1 / 30);
    expect(stepFrame(last, 1, 30, 10)).toBe(last);
  });
});
