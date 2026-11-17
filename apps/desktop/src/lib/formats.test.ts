import { describe, expect, it } from "vitest";
import type { MediaInfo } from "./api";
import {
  DEFAULTS,
  defaultFormat,
  defaultOptions,
  fitOptions,
  fpsChoices,
  initialOptions,
  RECIPES,
  recipeFrom,
  repacksJpeg,
  sizeChoices,
} from "./formats";

const clip: MediaInfo = {
  path: "/a/clip.mov",
  fileName: "clip.mov",
  extension: "mov",
  sizeBytes: 1,
  kind: "video",
  durationSecs: 10,
  width: 640,
  height: 360,
  fps: 24,
  videoCodec: "h264",
  audioCodec: "aac",
  reconstructsJpeg: false,
  tileGrid: null,
};

const all = () => true;

describe("formats", () => {
  it("only offers downscales and slower frame rates", () => {
    expect(sizeChoices(clip)).toEqual([240]);
    expect(fpsChoices(clip)).toEqual([15, 10]);
  });

  it("never defaults to the input's own format", () => {
    expect(defaultFormat({ ...clip, extension: "mp4" }, all)).toBe("webm");
    expect(defaultFormat({ ...clip, kind: "image", extension: "png" }, all)).toBe("jpg");
    expect(defaultFormat({ ...clip, kind: "audio", extension: "mp3" }, all)).toBe("m4a");
    expect(defaultFormat(clip, (f) => f !== "mp4")).toBe("webm");
  });

  it("applies recipes but drops settings that would upscale", () => {
    const gif = RECIPES.find((r) => r.id === "gif") ?? null;
    const options = initialOptions(clip, all, gif);
    expect(options.format).toBe("gif");
    expect(options.maxSize).toBeNull(); // 480 > 360
    expect(options.fps).toBe(15);

    const forImage = initialOptions({ ...clip, kind: "image" }, all, gif);
    expect(forImage.format).toBe("png");
  });

  it("starts from the user's defaults when they fit the file", () => {
    const defaults = {
      formats: { video: "webm", audio: null, image: "webp" },
      quality: 80,
      keepMetadata: false,
    } as const;
    const options = initialOptions(clip, all, null, {
      ...defaults,
      formats: { ...defaults.formats },
    });
    expect(options).toMatchObject({ format: "webm", quality: 80, keepMetadata: false });

    // Not for a file that already is one, nor when the encoder is missing.
    const webp = { ...clip, kind: "image" as const, extension: "webp" };
    const custom = { ...defaults, formats: { ...defaults.formats } };
    expect(initialOptions(webp, all, null, custom).format).toBe("png");
    expect(initialOptions(clip, (f) => f !== "webm", null, custom).format).toBe("mp4");
    expect(initialOptions(clip, all, null, DEFAULTS).format).toBe("mp4");
  });

  it("fits one file's settings to another, or says they don't fit", () => {
    const base = {
      ...defaultOptions("webm"),
      trimStart: 2,
      maxSize: 240,
      fps: 15,
      rotate: 90 as const,
    };
    expect(fitOptions(clip, base, all)).toMatchObject({
      format: "webm",
      trimStart: null,
      maxSize: 240,
      fps: 15,
      rotate: 90,
    });
    // A smaller, slower clip keeps its own size and rate.
    const small = { ...clip, width: 320, height: 180, fps: 12 };
    expect(fitOptions(small, base, all)).toMatchObject({ maxSize: null, fps: null });
    // Wrong kind, missing encoder, or no sound to pull.
    expect(fitOptions({ ...clip, kind: "audio" }, base, all)).toBeNull();
    expect(fitOptions(clip, base, (f) => f !== "webm")).toBeNull();
    expect(fitOptions({ ...clip, audioCodec: null }, defaultOptions("mp3"), all)).toBeNull();
  });

  it("carries a crop shape to other files, but not a hand-drawn area", () => {
    const area = { x: 0.1, y: 0, width: 0.4, height: 1 };
    const square = { ...defaultOptions("mp4"), crop: { aspect: [1, 1] as [number, number], area } };
    expect(fitOptions(clip, square, all)?.crop).toEqual({ aspect: [1, 1], area: null });
    const free = { ...square, crop: { aspect: null, area } };
    expect(fitOptions(clip, free, all)?.crop).toBeNull();

    // Sizes are offered for the cropped picture: 9:16 of 640×360 is 203×360.
    const tall = { rotate: 0 as const, crop: { aspect: [9, 16] as [number, number], area: null } };
    expect(sizeChoices(clip, tall)).toEqual([]);
    expect(sizeChoices({ ...clip, width: 1920, height: 1080 }, tall)).toEqual([480, 360, 240]);
    // A size that would upscale the cropped picture is dropped for that file.
    expect(
      fitOptions(clip, { ...defaultOptions("mp4"), ...tall, maxSize: 240 }, all),
    ).toMatchObject({ maxSize: null, crop: tall.crop });

    const recipe = recipeFrom("custom-3", "Square", clip, { ...square, maxSize: 240 });
    expect(recipe.options.crop).toEqual({ aspect: [1, 1], area: null });
    expect(recipe.detail).toBe("1:1 crop · 240p · balanced");
    expect(initialOptions(clip, all, recipe).crop).toEqual({ aspect: [1, 1], area: null });
  });

  it("counts a crop as an edit, unless it keeps the whole picture", () => {
    const jpeg = { ...clip, kind: "image" as const, extension: "jpg" };
    const repack = {
      ...defaultOptions("jxl"),
      jxl: { lossless: true, effort: 7, progressive: false },
    };
    expect(repacksJpeg(jpeg, repack)).toBe(true);
    expect(repacksJpeg(jpeg, { ...repack, crop: { aspect: [1, 1], area: null } })).toBe(false);
    expect(repacksJpeg(jpeg, { ...repack, crop: { aspect: [16, 9], area: null } })).toBe(true);
  });

  it("saves editor settings as a recipe without the per-file edits", () => {
    const options = {
      ...defaultOptions("mp4"),
      trimStart: 3,
      rotate: 180 as const,
      maxSize: 240,
      keepAudio: false,
    };
    const recipe = recipeFrom("custom-1", "Tiny clips", clip, options);
    expect(recipe).toMatchObject({
      title: "Tiny clips",
      custom: true,
      route: "VIDEO → MP4",
      tone: "pink",
      accepts: ["video"],
      detail: "240p · no sound · balanced",
    });
    expect(recipe.options).not.toHaveProperty("trimStart");
    expect(recipe.options).not.toHaveProperty("rotate");
    expect(initialOptions(clip, all, recipe)).toMatchObject({
      format: "mp4",
      maxSize: 240,
      keepAudio: false,
      rotate: 0,
      trimStart: null,
    });

    const sound = recipeFrom("custom-2", "Podcast", clip, {
      ...defaultOptions("mp3"),
      quality: 10,
    });
    expect(sound).toMatchObject({
      route: "ANY → MP3",
      accepts: ["video", "audio"],
      detail: "Tiny file",
    });
  });
});
