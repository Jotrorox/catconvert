import { describe, expect, it } from "vitest";
import type { ConvertOptions, MediaInfo, OutputFormat } from "./api";
import {
  describeEstimate,
  describeQuality,
  estimateBytes,
  estimateSequenceVideoBitrate,
  estimateVideoBitrate,
} from "./estimate";
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
  sampleRate: 48000,
  reconstructsJpeg: false,
  hdr: false,
  hasLocation: false,
  locationTags: [],
  tileGrid: null,
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
  sampleRate: null,
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
  sampleRate: 48000,
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
      [clip, "mkv"],
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

  it("expects AV1 to be smaller than H.264 and to grow with quality", () => {
    const av1 = { videoCodec: "av1" as const };
    for (const format of ["mp4", "mkv"] as const) {
      expect(size(clip, format, av1), format).toBeLessThan(size(clip, format) * 0.8);
      expect(size(clip, format, av1), format).toBeGreaterThan(size(clip, format) * 0.4);
    }
    expect(size(clip, "webm", { ...av1, quality: 0 })).toBeLessThan(size(clip, "webm", av1));
    expect(size(clip, "webm", av1)).toBeLessThan(size(clip, "webm", { ...av1, quality: 100 }));
    // MOV can't hold AV1, so the choice doesn't count there.
    expect(size(clip, "mov", av1)).toBe(size(clip, "mov"));
  });

  it("follows trims, dropped sound, frame rate and size", () => {
    const full = size(clip, "mp4");
    expect(size(clip, "mp4", { trimStart: 0, trimEnd: 21 })).toBeCloseTo(full / 2);
    expect(size(clip, "mp4", { keepAudio: false })).toBeLessThan(full);
    expect(size(clip, "mp4", { fps: 15 })).toBeLessThan(full);
    expect(size(clip, "mp4", { maxSize: 480 })).toBeLessThan(size(clip, "mp4", { maxSize: 720 }));
    // A 9:16 crop keeps under a third of a 16:9 picture; smaller frames cost
    // more bits per pixel, so the file shrinks a little less than that.
    const vertical = size(clip, "mp4", { keepAudio: false, crop: { aspect: [9, 16], area: null } });
    expect(vertical).toBeLessThan(size(clip, "mp4", { keepAudio: false }) / 2);
  });

  it("follows the speed and direction of animations, not of videos", () => {
    const gif = size(clip, "gif");
    // 2× halves the length; the 15 fps cap keeps the frame rate.
    expect(size(clip, "gif", { speed: 2 })).toBeCloseTo(gif / 2);
    expect(size(clip, "gif", { speed: 2, direction: "boomerang" })).toBeCloseTo(gif);
    expect(size(clip, "gif", { direction: "reverse" })).toBeCloseTo(gif);
    // At 0.25× a 30 fps clip plays at 7.5 fps for four times as long.
    expect(size(clip, "gif", { speed: 0.25 })).toBeCloseTo(gif * 2);
    expect(size(clip, "mp4", { speed: 2, direction: "boomerang" })).toBeCloseTo(size(clip, "mp4"));
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

describe("estimateVideoBitrate", () => {
  it("is the picture's share of the size estimate", () => {
    for (const quality of [0, 60, 100]) {
      const silent = opts("mp4", { quality, keepAudio: false });
      const rate = estimateVideoBitrate(clip, silent) ?? Number.NaN;
      expect((rate * 42) / 8).toBeCloseTo(estimateBytes(clip, silent) ?? Number.NaN);
    }
    expect(estimateVideoBitrate(clip, opts("mp4", { quality: 90 }))).toBeGreaterThan(
      estimateVideoBitrate(clip, opts("mp4", { quality: 30 })) ?? Number.POSITIVE_INFINITY,
    );
  });

  it("is exact with an exact bitrate, which sizes the file too", () => {
    const exact = opts("mp4", { videoBitrate: 2500, keepAudio: false });
    expect(estimateVideoBitrate(clip, exact)).toBe(2_500_000);
    expect(estimateBytes(clip, exact)).toBeCloseTo((2_500_000 * 42) / 8);
  });

  it("only applies to video files", () => {
    expect(estimateVideoBitrate(clip, opts("gif"))).toBeNull();
    expect(estimateVideoBitrate(clip, opts("mp3"))).toBeNull();
    expect(estimateVideoBitrate(photo, opts("png"))).toBeNull();
  });

  it("reads how busy the source is from its own video bitrate when known", () => {
    const busy = { ...clip, videoBitrate: 30_000_000 };
    const calm = { ...clip, videoBitrate: 1_000_000 };
    const at = (info: MediaInfo) => estimateVideoBitrate(info, opts("mp4")) ?? Number.NaN;
    expect(at(busy)).toBeGreaterThan(at(clip));
    expect(at(calm)).toBeLessThan(at(clip));
  });

  it("averages a slideshow over how long it plays", () => {
    const slideshow = opts("mp4");
    const rate = estimateSequenceVideoBitrate([photo, photo], slideshow, {
      frameSecs: 0.5,
      fit: "pad",
    });
    expect(rate).toBeGreaterThan(0);
    expect(
      estimateSequenceVideoBitrate([photo, photo], slideshow, { frameSecs: 1, fit: "pad" }),
    ).toBeCloseTo((rate ?? 0) / 2);
    expect(
      estimateSequenceVideoBitrate([photo], opts("gif"), { frameSecs: 1, fit: "pad" }),
    ).toBeNull();
  });
});

describe("describeQuality", () => {
  const h264 = { codec: "h264", crf: 24, kbps: null, passes: 1 } as const;

  it("puts the core's numbers and the estimate in one line", () => {
    expect(describeQuality({ audio: { mode: "bitrate", kbps: 128 }, video: h264 }, 3_840_000)).toBe(
      "Audio 128 kbps · Video ~3.8 Mbps (CRF 24)",
    );
    expect(describeQuality({ audio: null, video: h264 }, null)).toBe("Video CRF 24");
    expect(
      describeQuality(
        { audio: null, video: { ...h264, crf: null, kbps: 4000, passes: 2 } },
        4_000_000,
      ),
    ).toBe("Video 4 Mbps (2-pass)");
    expect(
      describeQuality(
        { audio: null, video: { codec: "av1", crf: null, kbps: 2500, passes: 1 } },
        null,
      ),
    ).toBe("Video 2.5 Mbps (average)");
  });

  it("names what lossless and uncompressed sound come to", () => {
    const audio = (rate: Parameters<typeof describeQuality>[0] & object) =>
      describeQuality(rate, null);
    expect(audio({ audio: { mode: "vbr", level: 3, kbps: 175 }, video: null })).toBe(
      "Audio ~175 kbps (VBR V3)",
    );
    expect(audio({ audio: { mode: "lossless" }, video: null })).toBe("Audio lossless");
    expect(audio({ audio: { mode: "pcm", kbps: 1536 }, video: null })).toBe("Audio 1.5 Mbps (PCM)");
    expect(audio({ audio: { mode: "pcm", kbps: null }, video: null })).toBe("Audio uncompressed");
    expect(audio({ audio: null, video: null })).toBeNull();
    expect(describeQuality(null, 1)).toBeNull();
  });
});
