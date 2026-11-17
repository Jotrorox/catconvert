import { describe, expect, it } from "vitest";
import type { ConvertOptions, MediaInfo, OutputFormat } from "./api";
import { describeEstimate, estimateBytes } from "./estimate";
import { defaultOptions } from "./formats";

const clip: MediaInfo = {
  path: "/a/clip.mov",
  fileName: "clip.mov",
  extension: "mov",
  sizeBytes: 48_000_000,
  kind: "video",
  durationSecs: 42,
  width: 1920,
  height: 1080,
  fps: 30,
  videoCodec: "h264",
  audioCodec: "aac",
  reconstructsJpeg: false,
};

const photo: MediaInfo = {
  ...clip,
  path: "/a/cat.jpg",
  fileName: "cat.jpg",
  extension: "jpg",
  sizeBytes: 3_000_000,
  kind: "image",
  durationSecs: null,
  width: 4032,
  height: 3024,
  fps: null,
  videoCodec: "mjpeg",
  audioCodec: null,
};

const song: MediaInfo = {
  ...clip,
  path: "/a/purr.wav",
  fileName: "purr.wav",
  extension: "wav",
  sizeBytes: 44_100 * 4 * 180,
  kind: "audio",
  durationSecs: 180,
  width: null,
  height: null,
  fps: null,
  videoCodec: null,
  audioCodec: "pcm_s16le",
};

const opts = (format: OutputFormat, patch: Partial<ConvertOptions> = {}): ConvertOptions => ({
  ...defaultOptions(format),
  ...patch,
});

const size = (info: MediaInfo, format: OutputFormat, patch: Partial<ConvertOptions> = {}) =>
  estimateBytes(info, opts(format, patch)) ?? Number.NaN;

describe("estimateBytes", () => {
  it("grows with quality for every lossy format", () => {
    const cases: [MediaInfo, OutputFormat][] = [
      [clip, "mp4"],
      [clip, "webm"],
      [clip, "gif"],
      [clip, "jxl"],
      [song, "mp3"],
      [song, "m4a"],
      [song, "ogg"],
      [photo, "jpg"],
      [photo, "webp"],
      [photo, "avif"],
      [photo, "jxl"],
    ];
    for (const [info, format] of cases) {
      const low = size(info, format, { quality: 0 });
      const mid = size(info, format, { quality: 50 });
      const high = size(info, format, { quality: 100 });
      expect(low, format).toBeLessThan(mid);
      expect(mid, format).toBeLessThan(high);
    }
  });

  it("lands in a believable range for a 1080p clip", () => {
    const mp4 = size(clip, "mp4");
    expect(mp4).toBeGreaterThan(10e6);
    expect(mp4).toBeLessThan(40e6);
    // Sharing-size 720p is much smaller.
    expect(size(clip, "mp4", { maxSize: 720, quality: 40 })).toBeLessThan(mp4 / 2);
  });

  it("follows trims, dropped sound, frame rate and size", () => {
    const full = size(clip, "mp4");
    expect(size(clip, "mp4", { trimStart: 0, trimEnd: 21 })).toBeCloseTo(full / 2);
    expect(size(clip, "mp4", { keepAudio: false })).toBeLessThan(full);
    expect(size(clip, "mp4", { fps: 15 })).toBeLessThan(full);
    expect(size(clip, "mp4", { maxSize: 480 })).toBeLessThan(size(clip, "mp4", { maxSize: 720 }));
  });

  it("uses the source for lossless sound", () => {
    expect(size(song, "wav")).toBeCloseTo(song.sizeBytes);
    expect(size(song, "flac")).toBeCloseTo(song.sizeBytes * 0.6);
    // MP3 V3 at the default quality, about 175 kbps.
    expect(size(song, "mp3")).toBeCloseTo((175_000 * 180) / 8);
  });

  it("knows a JPEG repacked into JPEG XL is about a fifth smaller", () => {
    const repack = opts("jxl", { jxl: { lossless: true, effort: 7, progressive: false } });
    expect(estimateBytes(photo, repack)).toBeCloseTo(photo.sizeBytes * 0.8);
  });

  it("gives up without the facts it needs", () => {
    expect(estimateBytes({ ...clip, durationSecs: null }, opts("mp4"))).toBeNull();
    expect(estimateBytes({ ...photo, width: null, height: null }, opts("png"))).toBeNull();
    expect(estimateBytes({ ...clip, audioCodec: null }, opts("mp3"))).toBeNull();
  });
});

describe("describeEstimate", () => {
  it("compares the guess with the original", () => {
    expect(describeEstimate(song, opts("wav"))).toBe("About 32 MB · about the same size");
    expect(describeEstimate(song, opts("flac"))).toBe("About 19 MB · 40% smaller");
    expect(describeEstimate(song, opts("ogg", { quality: 0 }))).toMatch(
      /^About 1\.4 MB · 22× smaller$/,
    );
    expect(describeEstimate(photo, opts("png"))).toMatch(/× bigger$/);
  });
});
