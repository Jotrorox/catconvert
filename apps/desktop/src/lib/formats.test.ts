import { describe, expect, it } from "vitest";
import type { MediaInfo } from "./api";
import {
  DEFAULTS,
  defaultFormat,
  defaultOptions,
  fitOptions,
  fpsChoices,
  initialOptions,
  keepsLocation,
  keepsMetadata,
  locationLine,
  metadataFrom,
  RECIPES,
  recipeDetail,
  recipeFrom,
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
  hasLocation: false,
  locationTags: [],
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
      metadata: "none",
    } as const;
    const options = initialOptions(clip, all, null, {
      ...defaults,
      formats: { ...defaults.formats },
    });
    expect(options).toMatchObject({ format: "webm", quality: 80, metadata: "none" });

    // Not for a file that already is one, nor when the encoder is missing.
    const webp = { ...clip, kind: "image" as const, extension: "webp" };
    const custom = { ...defaults, formats: { ...defaults.formats } };
    expect(initialOptions(webp, all, null, custom).format).toBe("png");
    expect(initialOptions(clip, (f) => f !== "webm", null, custom).format).toBe("mp4");
    expect(initialOptions(clip, all, null, DEFAULTS).format).toBe("mp4");
  });

  it("drops the location by default and reads the old on/off setting", () => {
    expect(DEFAULTS.metadata).toBe("noLocation");
    expect(initialOptions(clip, all, null).metadata).toBe("noLocation");
    expect(metadataFrom({ keepMetadata: true })).toBe("keep");
    expect(metadataFrom({ keepMetadata: false })).toBe("none");
    expect(metadataFrom({ metadata: "noLocation", keepMetadata: true })).toBe("noLocation");
    expect(metadataFrom({ metadata: "everything" })).toBeNull();
    expect(metadataFrom({})).toBeNull();
  });

  it("knows which outputs can carry metadata and a location", () => {
    expect(keepsMetadata("png", "image")).toBe(true);
    expect(keepsMetadata("jxl", "image")).toBe(true);
    for (const format of ["jpg", "webp", "avif"] as const) {
      expect(keepsMetadata(format, "image")).toBe(false);
    }
    expect(keepsMetadata("png", "video")).toBe(false); // a still from a clip
    expect(keepsMetadata("gif", "video")).toBe(false);
    expect(keepsMetadata("jxl", "video")).toBe(false);
    expect(keepsMetadata("mp4", "video")).toBe(true);
    expect(keepsMetadata("m4a", "video")).toBe(true);

    const photo: MediaInfo = { ...clip, kind: "image", extension: "jpg", hasLocation: true };
    const keep = { ...defaultOptions("png"), metadata: "keep" } as const;
    expect(locationLine(photo)).toBe("This photo knows where it was taken");
    expect(locationLine({ ...clip, hasLocation: true })).toBe(
      "This video knows where it was filmed",
    );
    expect(locationLine(clip)).toBeNull();
    expect(keepsLocation(photo, keep)).toBe(true);
    expect(keepsLocation(photo, { ...keep, metadata: "noLocation" })).toBe(false);
    expect(keepsLocation(photo, { ...keep, format: "jpg" })).toBe(false);
    expect(keepsLocation({ ...photo, hasLocation: false }, keep)).toBe(false);
  });

  it("mentions metadata in recipe details only when it isn't the default", () => {
    const options = defaultOptions("mp4");
    expect(recipeDetail(clip, options)).toBe("Balanced");
    expect(recipeDetail(clip, { ...options, metadata: "keep" })).toBe("Balanced · all metadata");
    expect(recipeDetail(clip, { ...options, metadata: "none" })).toBe("Balanced · no metadata");
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
