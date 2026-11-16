/**
 * What the UI knows about output formats, edit defaults and recipes. The
 * backend validates everything again; this only decides what to offer.
 */
import type { ConvertOptions, MediaInfo, MediaKind, OutputFormat } from "./api";

export type Tone = "pink" | "green" | "yellow" | "lavender";

export type FormatGroup = {
  kind: Exclude<MediaKind, "unknown">;
  label: string;
  tone: Tone;
  formats: readonly OutputFormat[];
};

/** JPEG XL appears twice: as an animation for videos, as a still for pictures. */
export const FORMAT_GROUPS: readonly FormatGroup[] = [
  {
    kind: "video",
    label: "Video",
    tone: "pink",
    formats: ["mp4", "webm", "mov", "mkv", "gif", "jxl"],
  },
  { kind: "audio", label: "Sound", tone: "yellow", formats: ["mp3", "m4a", "wav", "flac", "ogg"] },
  {
    kind: "image",
    label: "Picture",
    tone: "lavender",
    formats: ["png", "jpg", "webp", "avif", "jxl"],
  },
];

/** What writing `format` from an `input` file makes. JPEG XL moves when the input does. */
export function formatKind(format: OutputFormat, input: MediaKind): FormatGroup["kind"] {
  if (format === "jxl") return input === "video" ? "video" : "image";
  return FORMAT_GROUPS.find((group) => group.formats.includes(format))?.kind ?? "video";
}

/** Formats that become animated images rather than video files (no sound). */
export function isAnimatedImage(format: OutputFormat, input: MediaKind): boolean {
  return format === "gif" || (format === "jxl" && input === "video");
}

/**
 * Size and frame-rate caps the backend applies to animations when the user
 * keeps "Auto". Mirrors GIF_MAX_SIZE/GIF_FPS and ANIMATED_JXL_* in convert.rs.
 */
export function autoLimits(
  format: OutputFormat,
  input: MediaKind,
): { size: number; fps: number } | null {
  if (format === "gif") return { size: 480, fps: 15 };
  if (format === "jxl" && input === "video") return { size: 720, fps: 30 };
  return null;
}

/** Format groups that make sense for an input: video → video or sound, etc. */
export function groupsFor(kind: MediaKind): FormatGroup[] {
  switch (kind) {
    case "video":
      return FORMAT_GROUPS.filter((g) => g.kind !== "image");
    case "audio":
      return FORMAT_GROUPS.filter((g) => g.kind === "audio");
    case "image":
      return FORMAT_GROUPS.filter((g) => g.kind === "image");
    default:
      return [];
  }
}

/** Formats whose quality setting changes nothing. */
export const LOSSLESS: ReadonlySet<OutputFormat> = new Set(["wav", "flac", "png"]);

/** Whether these options keep every pixel/sample, so quality changes nothing. */
export function isLossless(options: ConvertOptions): boolean {
  return LOSSLESS.has(options.format) || (options.format === "jxl" && options.jxl.lossless);
}

export const label = (format: OutputFormat) => format.toUpperCase();

/** JPEG XL effort presets (cjxl's -e). */
export const JXL_EFFORTS = [
  { effort: 3, label: "Quick" },
  { effort: 7, label: "Normal" },
  { effort: 9, label: "Thorough" },
] as const;

/** Inputs that are already lossless (or a JPEG that cjxl can repack exactly). */
const LOSSLESS_BY_DEFAULT = new Set(["jpg", "jpeg", "png", "apng", "gif"]);

/** Whether the options leave the picture as it is: no trim, turn, mirror, resize or new rate. */
export function isUntouched(info: MediaInfo, options: ConvertOptions): boolean {
  const untrimmed =
    info.kind === "image" || (options.trimStart === null && options.trimEnd === null);
  return (
    untrimmed &&
    options.rotate === 0 &&
    !options.flip &&
    (options.maxSize === null || !sizeChoices(info).includes(options.maxSize)) &&
    (options.fps === null || !fpsChoices(info).includes(options.fps))
  );
}

/** A JPEG XL that holds a recompressed JPEG, turned back into that exact JPEG. */
export function restoresJpeg(info: MediaInfo, options: ConvertOptions): boolean {
  return info.reconstructsJpeg && options.format === "jpg" && isUntouched(info, options);
}

/** An unedited JPEG written as lossless JPEG XL: repacked, not re-encoded. */
export function repacksJpeg(info: MediaInfo, options: ConvertOptions): boolean {
  const jpeg = info.extension === "jpg" || info.extension === "jpeg";
  return jpeg && options.format === "jxl" && options.jxl.lossless && isUntouched(info, options);
}

export function defaultFormat(info: MediaInfo, available: (f: OutputFormat) => boolean) {
  const preferred: OutputFormat[] =
    info.kind === "video"
      ? ["mp4", "webm", "mov"]
      : info.kind === "audio"
        ? info.extension === "mp3"
          ? ["m4a", "ogg", "wav"]
          : ["mp3", "m4a", "ogg"]
        : info.extension === "png"
          ? ["jpg", "webp", "png"]
          : ["png", "jpg", "webp"];
  const offered = groupsFor(info.kind).flatMap((g) => g.formats);
  return (
    preferred.find((f) => available(f) && f !== info.extension) ??
    offered.find(available) ??
    preferred[0] ??
    "mp4"
  );
}

export function defaultOptions(format: OutputFormat): ConvertOptions {
  return {
    format,
    trimStart: null,
    trimEnd: null,
    maxSize: null,
    fps: null,
    rotate: 0,
    flip: false,
    quality: 60,
    keepAudio: true,
    jxl: { lossless: false, effort: 7, progressive: false },
  };
}

/** Short side of the picture after rotation metadata, if known. */
export function shortSide(info: MediaInfo): number | null {
  return info.width && info.height ? Math.min(info.width, info.height) : null;
}

const SIZES = [2160, 1440, 1080, 720, 480, 360, 240] as const;

/** Downscale choices smaller than the source. */
export function sizeChoices(info: MediaInfo): number[] {
  const short = shortSide(info);
  return SIZES.filter((size) => short === null || size < short);
}

export function sizeLabel(size: number, kind: MediaKind): string {
  if (kind === "image") return `${size} px`;
  return size === 2160 ? "4K" : `${size}p`;
}

const FRAME_RATES = [60, 30, 24, 15, 10] as const;

/** Frame-rate choices lower than the source. */
export function fpsChoices(info: MediaInfo): number[] {
  return FRAME_RATES.filter((fps) => info.fps === null || fps < info.fps - 0.5);
}

export type RecipeId = "gif" | "shrink" | "audio" | "web";

export type Recipe = {
  id: RecipeId;
  title: string;
  detail: string;
  route: string;
  tone: Tone;
  /** Applied on top of the defaults when a matching file arrives. */
  options: Partial<ConvertOptions> & { format: OutputFormat };
  /** Input kinds the recipe works for. */
  accepts: readonly MediaKind[];
};

export const RECIPES: readonly Recipe[] = [
  {
    id: "gif",
    title: "Make a GIF",
    detail: "Loop a clip, 480px, 15 fps",
    route: "VIDEO → GIF",
    tone: "pink",
    options: { format: "gif", maxSize: 480, fps: 15 },
    accepts: ["video"],
  },
  {
    id: "shrink",
    title: "Shrink to share",
    detail: "720p, small enough for chats",
    route: "ANY → MP4",
    tone: "green",
    options: { format: "mp4", maxSize: 720, quality: 40 },
    accepts: ["video"],
  },
  {
    id: "audio",
    title: "Pull the audio",
    detail: "Keep just the sound",
    route: "VIDEO → MP3",
    tone: "yellow",
    options: { format: "mp3", quality: 70 },
    accepts: ["video", "audio"],
  },
  {
    id: "web",
    title: "Web friendly",
    detail: "Small, fast, plays anywhere",
    route: "ANY → WEBM",
    tone: "lavender",
    options: { format: "webm", maxSize: 1080, quality: 50 },
    accepts: ["video"],
  },
];

/** Starting options for a freshly opened file, honouring a chosen recipe when it fits. */
export function initialOptions(
  info: MediaInfo,
  available: (f: OutputFormat) => boolean,
  recipe: Recipe | null,
): ConvertOptions {
  const jxl = {
    ...defaultOptions("jxl").jxl,
    lossless: LOSSLESS_BY_DEFAULT.has(info.extension ?? ""),
  };
  if (recipe?.accepts.includes(info.kind) && available(recipe.options.format)) {
    const options = { ...defaultOptions(recipe.options.format), jxl, ...recipe.options };
    // Drop choices that would upscale or speed up this particular file.
    if (options.maxSize !== null && !sizeChoices(info).includes(options.maxSize)) {
      options.maxSize = null;
    }
    if (options.fps !== null && !fpsChoices(info).includes(options.fps)) options.fps = null;
    return options;
  }
  return { ...defaultOptions(defaultFormat(info, available)), jxl };
}
