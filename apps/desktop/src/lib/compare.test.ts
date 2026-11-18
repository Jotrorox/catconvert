import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConvertOptions, MediaInfo, Sample } from "./api";
import {
  canCompare,
  clampPan,
  convertedKey,
  convertedName,
  convertedSize,
  createSample,
  originalKey,
  panAround,
  type SampleRequest,
  scaleFor,
  sourceTime,
} from "./compare";
import { defaultOptions } from "./formats";

const PHOTO: MediaInfo = {
  path: "/photos/cat.jpg",
  fileName: "cat.jpg",
  extension: "jpg",
  sizeBytes: 4_200_000,
  kind: "image",
  durationSecs: null,
  width: 4032,
  height: 3024,
  fps: null,
  videoCodec: "mjpeg",
  audioCodec: null,
  sampleRate: null,
  audioChannels: null,
  bitrate: null,
  videoBitrate: null,
  audioBitrate: null,
  reconstructsJpeg: false,
  hdr: false,
  hasLocation: false,
  locationTags: [],
  tileGrid: null,
};

const VIDEO: MediaInfo = {
  ...PHOTO,
  path: "/clips/clip.mov",
  fileName: "clip.mov",
  extension: "mov",
  kind: "video",
  durationSecs: 42,
  width: 1920,
  height: 1080,
  fps: 30,
  videoCodec: "h264",
  audioCodec: "aac",
};

const options = (patch: Partial<ConvertOptions>): ConvertOptions => ({
  ...defaultOptions(patch.format ?? "webp"),
  ...patch,
});

describe("labels", () => {
  it("names the converted half by format, codec and quality", () => {
    expect(convertedName(PHOTO, options({ format: "webp", quality: 73 }))).toBe("WEBP q73");
    expect(convertedName(PHOTO, options({ format: "png" }))).toBe("PNG");
    expect(
      convertedName(
        PHOTO,
        options({ format: "jxl", jxl: { lossless: true, effort: 7, progressive: false } }),
      ),
    ).toBe("JXL lossless");
    expect(convertedName(VIDEO, options({ format: "mp4", quality: 60 }))).toBe("MP4 q60");
    expect(convertedName(VIDEO, options({ format: "mp4", videoCodec: "h265", quality: 40 }))).toBe(
      "MP4 H.265 q40",
    );
    expect(convertedName(VIDEO, options({ format: "webm", videoBitrate: 4000 }))).toBe(
      "WEBM 4 Mbps",
    );
    // Animations have no codec choice.
    expect(convertedName(VIDEO, options({ format: "gif", quality: 50 }))).toBe("GIF q50");
  });

  it("gives an exact size for a whole sample, a rough one otherwise", () => {
    const webp = options({ format: "webp", quality: 73 });
    expect(convertedSize(PHOTO, webp, { bytes: 612_345, exact: true })).toBe("612 KB");
    expect(convertedSize(PHOTO, webp, { bytes: 50_000, exact: false })).toMatch(/^~[\d.]+ [KM]B$/);
    expect(convertedSize(PHOTO, webp, null)).toMatch(/^~/);
    expect(convertedSize({ ...PHOTO, width: null, height: null }, webp, null)).toBeNull();
  });

  it("only compares things with a picture", () => {
    expect(canCompare(PHOTO, "avif")).toBe(true);
    expect(canCompare(VIDEO, "gif")).toBe(true);
    expect(canCompare(VIDEO, "mp3")).toBe(false);
    expect(canCompare({ kind: "audio" }, "wav")).toBe(false);
  });
});

describe("sample keys", () => {
  const base = options({ format: "webp", quality: 60 });

  it("asks for the original again only when the picture changes", () => {
    const key = originalKey(PHOTO, base, 0, 1080);
    expect(originalKey(PHOTO, { ...base, quality: 10, format: "avif" }, 3, 1080)).toBe(key);
    expect(originalKey(PHOTO, { ...base, rotate: 90 }, 0, 1080)).not.toBe(key);
    expect(originalKey(PHOTO, base, 0, null)).not.toBe(key);
    // A video's frame depends on the time and the frame rate.
    const clip = originalKey(VIDEO, base, 1, 1080);
    expect(originalKey(VIDEO, base, 2, 1080)).not.toBe(clip);
    expect(originalKey(VIDEO, { ...base, fps: 15 }, 1, 1080)).not.toBe(clip);
  });

  it("asks for the converted half again when the encoding changes", () => {
    const key = convertedKey(PHOTO, base, 0, 1080);
    expect(convertedKey(PHOTO, { ...base, quality: 61 }, 0, 1080)).not.toBe(key);
    expect(
      convertedKey(
        PHOTO,
        { ...base, trimStart: 2, keepAudio: false, destination: { kind: "replaceInput" } },
        0,
        1080,
      ),
    ).toBe(key);
  });
});

describe("zoom and pan", () => {
  const stage = { width: 800, height: 500 };
  const picture = { width: 4000, height: 3000 };

  it("fits the picture in or shows it at 100% and 200%", () => {
    expect(scaleFor("fit", stage, picture)).toBeCloseTo(500 / 3000);
    expect(scaleFor(1, stage, picture)).toBe(1);
    expect(scaleFor(2, stage, picture)).toBe(2);
    expect(scaleFor("fit", stage, { width: 0, height: 0 })).toBe(1);
  });

  it("keeps a zoomed picture covering the stage", () => {
    expect(clampPan({ x: 5000, y: -5000 }, stage, picture)).toEqual({ x: 1600, y: -1250 });
    // A picture smaller than the stage stays centred.
    expect(clampPan({ x: 40, y: 40 }, stage, { width: 300, height: 200 })).toEqual({ x: 0, y: 0 });
  });

  it("zooms around the point under the pointer", () => {
    // The spot 100 px right of centre stays put going from 1× to 2×.
    expect(panAround({ x: 0, y: 0 }, { x: 100, y: 0 }, 1, 2)).toEqual({ x: -100, y: 0 });
    expect(panAround({ x: 0, y: 0 }, { x: 0, y: 0 }, 0.2, 1)).toEqual({ x: 0, y: 0 });
  });
});

describe("sourceTime", () => {
  const plain = { trimStart: 10, trimEnd: 20, speed: 1, direction: "forward" } as const;

  it("maps a moment of the result to the original", () => {
    expect(sourceTime(plain, 42, 2.5)).toBe(12.5);
    expect(sourceTime({ ...plain, trimStart: null, trimEnd: null }, 42, 3)).toBe(3);
    expect(sourceTime({ ...plain, speed: 2 }, 42, 2.5)).toBe(15);
    expect(sourceTime({ ...plain, direction: "reverse" }, 42, 2.5)).toBe(17.5);
    // A boomerang plays forward, then back.
    expect(sourceTime({ ...plain, direction: "boomerang" }, 42, 4)).toBe(14);
    expect(sourceTime({ ...plain, direction: "boomerang" }, 42, 16)).toBe(14);
    // Never outside the kept range.
    expect(sourceTime(plain, 42, 99)).toBe(20);
  });
});

describe("createSample", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const sample = (url: string): Sample => ({ url, bytes: 1, exact: false });

  /** Resolve pending promises between timer steps. */
  const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };

  it("waits for changes to settle, runs one at a time and keeps recent samples", async () => {
    const pending: { key: string; resolve: (s: Sample) => void }[] = [];
    const run = vi.fn(
      (key: string) =>
        new Promise<Sample>((resolve) => pending.push({ key, resolve: (s) => resolve(s) })),
    );
    await createRoot(async (dispose) => {
      const [key, setKey] = createSignal<string | null>("a");
      const request = (): SampleRequest | null => {
        const k = key();
        return k ? { key: k, run: () => run(k) } : null;
      };
      const { state } = createSample(request, 100);
      await flush();
      expect(state().busy).toBe(true);
      expect(run).not.toHaveBeenCalled();

      // Quick changes only ask for the last one.
      setKey("b");
      await vi.advanceTimersByTimeAsync(100);
      expect(run.mock.calls.map((c) => c[0])).toEqual(["b"]);

      // While "b" renders, "c" then "d" are asked for; only "d" runs next.
      setKey("c");
      await vi.advanceTimersByTimeAsync(100);
      setKey("d");
      await vi.advanceTimersByTimeAsync(100);
      expect(run).toHaveBeenCalledTimes(1);
      pending[0]?.resolve(sample("b.png"));
      await flush();
      // "b" is stale: it is kept for later but not shown.
      expect(state().sample).toBeNull();
      expect(run.mock.calls.map((c) => c[0])).toEqual(["b", "d"]);
      pending[1]?.resolve(sample("d.png"));
      await flush();
      expect(state()).toEqual({ sample: sample("d.png"), busy: false, error: null });

      // Going back to "b" needs no new render.
      setKey("b");
      expect(state().sample?.url).toBe("b.png");
      expect(state().busy).toBe(false);
      expect(run).toHaveBeenCalledTimes(2);

      // Turning it off keeps the last picture but stops asking.
      setKey(null);
      expect(state().busy).toBe(false);
      dispose();
    });
  });

  it("keeps the last sample and reports a failure", async () => {
    await createRoot(async (dispose) => {
      const [fail, setFail] = createSignal(false);
      const { state } = createSample(
        () => ({
          key: fail() ? "bad" : "good",
          run: () => (fail() ? Promise.reject("no encoder") : Promise.resolve(sample("ok.png"))),
        }),
        10,
      );
      await vi.advanceTimersByTimeAsync(10);
      await flush();
      expect(state().sample?.url).toBe("ok.png");
      setFail(true);
      await vi.advanceTimersByTimeAsync(10);
      await flush();
      expect(state()).toEqual({ sample: sample("ok.png"), busy: false, error: "no encoder" });
      dispose();
    });
  });

  it("asks once more for a picture that didn't load", async () => {
    await createRoot(async (dispose) => {
      const run = vi.fn(async () => sample("again.png"));
      const { retry } = createSample(() => ({ key: "k", run }), 10);
      await vi.advanceTimersByTimeAsync(10);
      await flush();
      expect(run).toHaveBeenCalledTimes(1);
      retry();
      await vi.advanceTimersByTimeAsync(10);
      await flush();
      expect(run).toHaveBeenCalledTimes(2);
      // Not forever.
      retry();
      await vi.advanceTimersByTimeAsync(10);
      await flush();
      expect(run).toHaveBeenCalledTimes(2);
      dispose();
    });
  });
});
