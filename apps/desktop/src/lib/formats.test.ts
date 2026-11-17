import { describe, expect, it } from "vitest";
import type { MediaInfo } from "./api";
import {
  autoLimits,
  DEFAULTS,
  defaultFormat,
  defaultOptions,
  fitOptions,
  formatKind,
  fpsChoices,
  initialOptions,
  isAnimatedImage,
  looksLikePicture,
  naturalOrder,
  playbackOrder,
  RECIPES,
  recipeFrom,
  sequenceFormat,
  sequenceFrame,
  sequenceName,
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

describe("pictures → animation", () => {
  const photo = (path: string, width = 4000, height = 3000): MediaInfo => ({
    ...clip,
    path,
    fileName: path.split("/").pop() ?? path,
    extension: "jpg",
    kind: "image",
    durationSecs: null,
    width,
    height,
    fps: null,
    videoCodec: "mjpeg",
    audioCodec: null,
  });

  it("treats WebP like JPEG XL: animated from video, still from pictures", () => {
    expect(formatKind("webp", "video")).toBe("video");
    expect(formatKind("webp", "image")).toBe("image");
    expect(isAnimatedImage("webp", "video")).toBe(true);
    expect(isAnimatedImage("webp", "image")).toBe(false);
    expect(autoLimits("webp", "video")).toEqual({ size: 480, fps: 24 });
    expect(autoLimits("webp", "image")).toBeNull();
  });

  it("only asks about drops that look like still pictures", () => {
    expect(looksLikePicture("/a/IMG_1.JPG")).toBe(true);
    expect(looksLikePicture("/a/scan.tiff")).toBe(true);
    expect(looksLikePicture("/a/loop.gif")).toBe(false);
    expect(looksLikePicture("/a/clip.mov")).toBe(false);
    expect(looksLikePicture("/a/README")).toBe(false);
  });

  it("orders names naturally and plays them back", () => {
    const names = ["cat 10.jpg", "Cat 9.jpg", "cat 1.jpg"].map((n) => photo(`/p/${n}`));
    expect(names.sort(naturalOrder).map((p) => p.fileName)).toEqual([
      "cat 1.jpg",
      "Cat 9.jpg",
      "cat 10.jpg",
    ]);
    expect(playbackOrder(4, "forward")).toEqual([0, 1, 2, 3]);
    expect(playbackOrder(4, "reverse")).toEqual([3, 2, 1, 0]);
    expect(playbackOrder(4, "boomerang")).toEqual([0, 1, 2, 3, 2, 1]);
    expect(playbackOrder(2, "boomerang")).toEqual([0, 1]);
  });

  it("sizes the frame like the backend", () => {
    const gif = { ...defaultOptions("gif") };
    expect(sequenceFrame(photo("/p/a.jpg"), gif)).toEqual({ width: 640, height: 480 });
    expect(sequenceFrame(photo("/p/a.jpg", 1080, 1920), { ...gif, format: "webp" })).toEqual({
      width: 480,
      height: 852,
    });
    expect(sequenceFrame(photo("/p/a.jpg", 1001, 701), defaultOptions("mp4"))).toEqual({
      width: 1000,
      height: 700,
    });
    expect(sequenceFrame(photo("/p/a.jpg"), { ...gif, rotate: 90 })).toEqual({
      width: 480,
      height: 640,
    });
    expect(sequenceFrame({ ...photo("/p/a.jpg"), width: null }, gif)).toBeNull();
  });

  it("names the animation after the pictures or their folder", () => {
    expect(sequenceName([photo("/p/cat 1.jpg"), photo("/p/cat 2.jpg")])).toBe("cat");
    expect(sequenceName([photo("/p/IMG_0012.jpg"), photo("/p/IMG_0013.jpg")])).toBe("IMG");
    expect(sequenceName([photo("/trip/dog.jpg"), photo("/trip/owl.jpg")])).toBe("trip");
    // Windows paths, and never a hidden file from a hidden folder.
    const windows = (name: string) => ({ ...photo(`C:\\.hidden\\${name}`), fileName: name });
    expect(sequenceName([windows("a.jpg"), windows("b.jpg")])).toBe("hidden");
  });

  it("starts with the first installed animation format", () => {
    expect(sequenceFormat(() => true)).toBe("gif");
    expect(sequenceFormat((f) => f === "mp4")).toBe("mp4");
  });
});
