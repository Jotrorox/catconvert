import { describe, expect, it } from "vitest";
import type { MediaInfo } from "./api";
import {
  autoLimits,
  codecFor,
  codecOf,
  DEFAULTS,
  defaultFormat,
  defaultOptions,
  describeStreams,
  destinationFor,
  fadeSpans,
  fitOptions,
  folderName,
  formatKind,
  fpsChoices,
  initialOptions,
  isAnimatedImage,
  isHuge,
  isUntouched,
  keepsLocation,
  keepsMetadata,
  locationLine,
  looksLikePicture,
  maxUpscaleSize,
  metadataFrom,
  naturalOrder,
  outputHasSound,
  outputSeconds,
  outputSize,
  playbackOrder,
  RECIPES,
  recipeDetail,
  recipeFrom,
  repacksJpeg,
  replacesOriginal,
  scalerFrom,
  sequenceFormat,
  sequenceFrame,
  sequenceName,
  sequenceOptions,
  sequenceUpscaleChoices,
  sizeChoices,
  upscaleChoices,
  upscaleFrom,
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
  sampleRate: 48000,
  reconstructsJpeg: false,
  hdr: false,
  hasLocation: false,
  locationTags: [],
  tileGrid: null,
};

const all = () => true;

describe("formats", () => {
  it("knows when the output carries sound", () => {
    expect(outputHasSound(clip, defaultOptions("mp4"))).toBe(true);
    expect(outputHasSound(clip, defaultOptions("mp3"))).toBe(true);
    expect(outputHasSound(clip, { ...defaultOptions("mp4"), keepAudio: false })).toBe(false);
    expect(outputHasSound(clip, defaultOptions("gif"))).toBe(false);
    expect(outputHasSound(clip, defaultOptions("jxl"))).toBe(false);
    expect(outputHasSound({ ...clip, audioCodec: null }, defaultOptions("mp4"))).toBe(false);
  });

  it("clips fades to the kept length like the planner", () => {
    const sound = { normalize: false, fadeIn: 3, fadeOut: 3 };
    expect(fadeSpans(sound, 10)).toEqual({ fadeIn: 3, fadeOut: 3 });
    expect(fadeSpans(sound, 2)).toEqual({ fadeIn: 2, fadeOut: 2 });
    // A fade-out needs to know where the end is.
    expect(fadeSpans(sound, null)).toEqual({ fadeIn: 3, fadeOut: 0 });
  });

  it("describes and keeps sound clean-up in recipes", () => {
    const options = {
      ...defaultOptions("mp3"),
      sound: { normalize: true, fadeIn: 1, fadeOut: 0 },
    };
    expect(recipeDetail(clip, options)).toBe("Balanced · even loudness · fades");
    // Muted video ignores the sound settings.
    const muted = { ...defaultOptions("mp4"), keepAudio: false, sound: options.sound };
    expect(recipeDetail(clip, muted)).toBe("No sound · balanced");
    const recipe = recipeFrom("r", "Podcast", clip, options);
    expect(recipe.options.sound).toEqual(options.sound);
    expect(recipe.options.sound).not.toBe(options.sound);
  });

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
      ...DEFAULTS,
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

  it("starts files where Settings says results go", () => {
    expect(initialOptions(clip, all, null).destination).toEqual({ kind: "nextToInput" });
    const folder = {
      ...DEFAULTS,
      output: "folder",
      outputFolder: "/out",
      outputByKind: true,
    } as const;
    expect(initialOptions(clip, all, null, folder).destination).toEqual({
      kind: "folder",
      path: "/out",
      byKind: true,
    });
    // A folder choice without a folder saves next to the original.
    expect(destinationFor({ ...folder, outputFolder: null })).toEqual({ kind: "nextToInput" });
    // The folder is kept for a one-off choice even when the default differs.
    expect(destinationFor({ ...folder, output: "nextToInput" }, "folder")).toEqual({
      kind: "folder",
      path: "/out",
      byKind: true,
    });

    const replace = { ...DEFAULTS, output: "replaceInput" } as const;
    const options = initialOptions(clip, all, RECIPES[0] ?? null, replace);
    expect(replacesOriginal(options)).toBe(true);
    // Fitting the settings to the next file keeps where they go.
    expect(fitOptions({ ...clip, path: "/b/other.mov" }, options, all)?.destination).toEqual({
      kind: "replaceInput",
    });
    // An animation can't replace its pictures.
    expect(sequenceOptions(() => true, replace).destination).toEqual({ kind: "nextToInput" });
    expect(sequenceOptions(() => true, folder).destination).toMatchObject({ kind: "folder" });
    // Recipes don't carry where results go.
    expect(recipeFrom("r", "Mine", clip, options).options).not.toHaveProperty("destination");

    expect(folderName("/Users/cat/Downloads/CatConvert/")).toBe("CatConvert");
    expect(folderName("C:\\Users\\cat\\Converted")).toBe("Converted");
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

  it("offers AV1 where the container can hold it", () => {
    expect(codecOf(defaultOptions("mp4"))).toBe("h264");
    expect(codecOf(defaultOptions("webm"))).toBe("vp9");
    expect(codecOf({ format: "mkv", videoCodec: "av1" })).toBe("av1");
    expect(codecOf({ format: "mov", videoCodec: "av1" })).toBe("h264");
    expect(codecOf({ format: "gif", videoCodec: "av1" })).toBeNull();
    // Switching formats keeps AV1 where it fits and falls back elsewhere.
    expect(codecFor("webm", "av1")).toBe("av1");
    expect(codecFor("mov", "av1")).toBeNull();
    expect(codecFor("mp4", "vp9")).toBeNull();
    const av1 = { ...defaultOptions("mp4"), videoCodec: "av1" as const };
    expect(recipeDetail(clip, av1)).toBe("AV1 · balanced");
    // An exact bitrate stands in for the quality level of video files.
    const exact = { ...defaultOptions("mp4"), videoBitrate: 4000 };
    expect(recipeDetail(clip, exact)).toBe("4 Mbps");
    expect(recipeDetail(clip, { ...exact, format: "mp3" })).toBe("Balanced");
    expect(fitOptions(clip, exact, all)?.videoBitrate).toBe(4000);
    expect(fitOptions(clip, av1, all)?.videoCodec).toBe("av1");
    expect(fitOptions(clip, { ...av1, format: "mov" }, all)?.videoCodec).toBeNull();
  });

  it("offers H.265 in MP4, MOV and MKV but not WebM", () => {
    for (const format of ["mp4", "mov", "mkv"] as const) {
      expect(codecOf({ format, videoCodec: "h265" }), format).toBe("h265");
      expect(codecFor(format, "h265"), format).toBe("h265");
    }
    expect(codecOf({ format: "webm", videoCodec: "h265" })).toBe("vp9");
    expect(codecFor("webm", "h265")).toBeNull();
    const h265 = { ...defaultOptions("mov"), videoCodec: "h265" as const };
    expect(recipeDetail(clip, h265)).toBe("H.265 · balanced");
    expect(recipeDetail(clip, { ...h265, hardware: true })).toBe("H.265 · hardware · balanced");
    // The hardware switch only means something for H.265.
    expect(recipeDetail(clip, { ...defaultOptions("mp4"), hardware: true })).toBe("Balanced");
    expect(fitOptions(clip, { ...h265, format: "webm" }, all)?.videoCodec).toBeNull();
  });

  it("times animations by their speed and direction", () => {
    const gif = { ...defaultOptions("gif"), trimStart: 2, trimEnd: 8 };
    expect(outputSeconds(clip, gif)).toBe(6);
    expect(outputSeconds(clip, { ...gif, speed: 2 })).toBe(3);
    expect(outputSeconds(clip, { ...gif, speed: 0.5, direction: "boomerang" })).toBe(24);
    // Ignored for ordinary video.
    expect(outputSeconds(clip, { ...defaultOptions("mp4"), speed: 4 })).toBe(10);
    // A reversed animation isn't the original, so no lossless shortcut.
    const jxl = defaultOptions("jxl");
    expect(isUntouched(clip, jxl)).toBe(true);
    expect(isUntouched(clip, { ...jxl, direction: "reverse" })).toBe(false);
    expect(isUntouched(clip, { ...defaultOptions("mp4"), direction: "reverse" })).toBe(true);
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

    const boomerang = recipeFrom("custom-3", "Bounce", clip, {
      ...defaultOptions("gif"),
      speed: 2,
      direction: "boomerang",
    });
    expect(boomerang.detail).toBe("2× · boomerang · balanced");
    expect(initialOptions(clip, all, boomerang)).toMatchObject({
      speed: 2,
      direction: "boomerang",
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

describe("upscaling", () => {
  const photo: MediaInfo = {
    ...clip,
    path: "/a/cat.jpg",
    fileName: "cat.jpg",
    extension: "jpg",
    kind: "image",
    durationSecs: null,
    width: 640,
    height: 480,
    fps: null,
    videoCodec: "mjpeg",
    audioCodec: null,
  };
  const hd: MediaInfo = { ...clip, width: 1280, height: 720 };
  const uhd: MediaInfo = { ...clip, width: 3840, height: 2160 };
  const png = defaultOptions("png");
  const mp4 = defaultOptions("mp4");

  it("enlarges by a factor or to a short side, never shrinking", () => {
    expect(outputSize(photo, { ...png, upscale: { factor: 2 } })).toEqual({
      width: 1280,
      height: 960,
    });
    expect(outputSize(photo, { ...png, upscale: { factor: 1.5 } })).toEqual({
      width: 960,
      height: 720,
    });
    expect(outputSize(hd, { ...mp4, upscale: { size: 1080 } })).toEqual({
      width: 1920,
      height: 1080,
    });
    // A target the picture already reaches changes nothing.
    expect(outputSize(uhd, { ...mp4, upscale: { size: 1080 } })).toEqual({
      width: 3840,
      height: 2160,
    });
    // Turned and cropped first, then enlarged.
    expect(
      outputSize(photo, {
        ...png,
        rotate: 90,
        crop: { aspect: [1, 1], area: null },
        upscale: { factor: 2 },
      }),
    ).toEqual({ width: 960, height: 960 });
  });

  it("keeps video sides even and caps the result with an explicit size", () => {
    const odd: MediaInfo = { ...clip, width: 641, height: 361 };
    expect(outputSize(odd, { ...mp4, upscale: { size: 721 } })).toEqual({
      width: 1278,
      height: 720,
    });
    expect(outputSize(hd, { ...mp4, upscale: { factor: 3 }, maxSize: 1080 })).toEqual({
      width: 1920,
      height: 1080,
    });
  });

  it("lifts an animation's default size cap only when upscaling", () => {
    const gif = defaultOptions("gif");
    expect(outputSize(hd, gif)).toEqual({ width: 852, height: 480 });
    expect(outputSize(hd, { ...gif, upscale: { factor: 2 } })).toEqual({
      width: 2560,
      height: 1440,
    });
    expect(outputSize(hd, { ...gif, upscale: { factor: 2 }, maxSize: 480 })).toEqual({
      width: 852,
      height: 480,
    });
  });

  it("offers presets that enlarge the file and fit the format", () => {
    expect(upscaleChoices(photo, png)).toEqual([
      { factor: 2 },
      { factor: 3 },
      { factor: 4 },
      { size: 720 },
      { size: 1080 },
      { size: 1440 },
      { size: 2160 },
    ]);
    expect(upscaleChoices(hd, mp4)).toEqual([
      { factor: 2 },
      { factor: 3 },
      { factor: 4 },
      { size: 1080 },
      { size: 1440 },
      { size: 2160 },
    ]);
    // Video encoders stop at 8192 px on the long side: 4K only doubles.
    expect(upscaleChoices(uhd, mp4)).toEqual([{ factor: 2 }]);
    expect(maxUpscaleSize(uhd, mp4)).toBe(4608);
    // A still from that video can go further.
    expect(upscaleChoices(uhd, defaultOptions("png"))).toEqual([
      { factor: 2 },
      { factor: 3 },
      { factor: 4 },
    ]);
    expect(sequenceUpscaleChoices(photo, defaultOptions("gif"))).toHaveLength(7);
  });

  it("warns about huge results only when upscaling makes them", () => {
    expect(isHuge(uhd, { ...mp4, upscale: { factor: 2 } })).toBe(true);
    expect(isHuge(hd, { ...mp4, upscale: { size: 2160 } })).toBe(false);
    expect(isHuge(uhd, mp4)).toBe(false);
    expect(isHuge(hd, { ...defaultOptions("gif"), upscale: { size: 1440 } })).toBe(true);
    expect(isHuge(photo, { ...png, upscale: { factor: 8 } })).toBe(false);
    const big: MediaInfo = { ...photo, width: 4032, height: 3024 };
    expect(isHuge(big, { ...png, upscale: { factor: 2 } })).toBe(false);
    expect(isHuge(big, { ...png, upscale: { factor: 3 } })).toBe(true);
  });

  it("counts an upscale as an edit and describes it in recipes", () => {
    expect(isUntouched(photo, { ...png, upscale: { factor: 2 } })).toBe(false);
    expect(isUntouched(photo, { ...png, upscale: { size: 480 } })).toBe(true);
    expect(recipeDetail(photo, { ...png, upscale: { factor: 2 } })).toBe("2× upscale · lossless");
    expect(recipeDetail(hd, { ...mp4, upscale: { size: 1080 } })).toBe(
      "Upscaled to 1080p · balanced",
    );
  });

  it("carries an upscale to other files unless it gets too big", () => {
    const available = () => true;
    const options = { ...mp4, upscale: { factor: 2 }, scaler: "spline" } as const;
    expect(fitOptions(hd, options, available)).toMatchObject({
      upscale: { factor: 2 },
      scaler: "spline",
    });
    expect(fitOptions(uhd, { ...mp4, upscale: { factor: 3 } }, available)?.upscale).toBeNull();
    // A target the file reaches stays, as it changes nothing...
    expect(fitOptions(uhd, { ...mp4, upscale: { size: 1080 } }, available)?.upscale).toEqual({
      size: 1080,
    });
    // ...except where it would lift an animation's default size cap.
    const gif = { ...defaultOptions("gif"), upscale: { size: 1080 } };
    expect(fitOptions(uhd, gif, available)?.upscale).toBeNull();
  });

  it("reads stored upscale settings, defaulting what's missing or wrong", () => {
    expect(upscaleFrom(undefined)).toBeNull();
    expect(upscaleFrom({ factor: 2 })).toEqual({ factor: 2 });
    expect(upscaleFrom({ size: 1080 })).toEqual({ size: 1080 });
    expect(upscaleFrom({ factor: 20 })).toBeNull();
    expect(upscaleFrom({ size: "big" })).toBeNull();
    expect(scalerFrom(undefined)).toBe("auto");
    expect(scalerFrom("spline")).toBe("spline");
    expect(scalerFrom("nearest")).toBe("auto");
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
    // An upscale lifts the default cap; an explicit size still applies.
    const small = photo("/p/a.jpg", 640, 480);
    expect(sequenceFrame(small, { ...gif, upscale: { factor: 2 } })).toEqual({
      width: 1280,
      height: 960,
    });
    expect(sequenceFrame(small, { ...gif, upscale: { size: 721 } })).toEqual({
      width: 960,
      height: 720,
    });
    expect(sequenceFrame(small, { ...gif, upscale: { factor: 4 }, maxSize: 1080 })).toEqual({
      width: 1440,
      height: 1080,
    });
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

describe("describeStreams", () => {
  it("shows each stream's codec with its bitrate", () => {
    const rated = { ...clip, videoBitrate: 12_000_000, audioBitrate: 256_000 };
    expect(describeStreams(rated)).toBe("H264 12 Mbps · AAC 256 kbps");
    expect(describeStreams({ ...rated, audioCodec: null, audioBitrate: null })).toBe(
      "H264 12 Mbps",
    );
    const song = { ...clip, kind: "audio" as const, videoCodec: null, audioBitrate: 320_000 };
    expect(describeStreams(song)).toBe("AAC 320 kbps");
  });

  it("falls back to the codecs and the whole file's bitrate", () => {
    expect(describeStreams(clip)).toBe("H264 · AAC");
    expect(describeStreams({ ...clip, bitrate: 900_000 })).toBe("H264 · AAC · 900 kbps total");
    expect(describeStreams({ ...clip, kind: "image", videoCodec: "mjpeg" })).toBeNull();
  });
});
