/**
 * What the UI knows about output formats, edit defaults and recipes. The
 * backend validates everything again; this only decides what to offer.
 */
import type { ConvertOptions, MediaInfo, MediaKind, MetadataMode, OutputFormat } from "./api";

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

/**
 * Whether the options leave the picture as it is: no trim, turn, mirror,
 * resize or new rate. Dropping metadata doesn't count; the backend cleans the
 * JPEG before libjxl repacks it (see scrub.rs).
 */
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
    metadata: "noLocation",
  };
}

export const METADATA_CHOICES: readonly { mode: MetadataMode; label: string }[] = [
  { mode: "keep", label: "Keep everything" },
  { mode: "noLocation", label: "Keep all but location" },
  { mode: "none", label: "Remove everything" },
];

/**
 * Whether the output can carry tags over at all. ffmpeg writes no EXIF into
 * JPG, WebP or AVIF, stills grabbed from a video have none, and animations
 * keep no tags, so those come out clean whatever the setting.
 */
export function keepsMetadata(format: OutputFormat, input: MediaKind): boolean {
  if (formatKind(format, input) === "image") {
    return input === "image" && (format === "png" || format === "jxl");
  }
  return !isAnimatedImage(format, input);
}

/** Whether the converted file will still say where the original was made. */
export function keepsLocation(info: MediaInfo, options: ConvertOptions): boolean {
  return (
    info.hasLocation && options.metadata === "keep" && keepsMetadata(options.format, info.kind)
  );
}

const PLACE_WORDS = {
  image: "photo knows where it was taken",
  video: "video knows where it was filmed",
  audio: "recording knows where it was made",
  unknown: "file knows where it was made",
} as const;

/** "📍 This photo knows where it was taken", for files with a location. */
export function locationLine(info: MediaInfo): string | null {
  return info.hasLocation ? `This ${PLACE_WORDS[info.kind]}` : null;
}

/**
 * The metadata setting as stored by older versions (`keepMetadata: boolean`)
 * or now. On only meant "keep", off meant "remove everything".
 */
export function metadataFrom(stored: {
  metadata?: unknown;
  keepMetadata?: unknown;
}): MetadataMode | null {
  if (
    stored.metadata === "keep" ||
    stored.metadata === "noLocation" ||
    stored.metadata === "none"
  ) {
    return stored.metadata;
  }
  if (typeof stored.keepMetadata === "boolean") return stored.keepMetadata ? "keep" : "none";
  return null;
}

export const QUALITY_STEPS = ["Tiny file", "Balanced", "Crisp"] as const;

export function qualityStep(quality: number): (typeof QUALITY_STEPS)[number] {
  return QUALITY_STEPS[quality < 34 ? 0 : quality > 66 ? 2 : 1];
}

/** What freshly opened files start with, set in Settings. */
export type Defaults = {
  /** Preferred output per input kind; `null` lets CatConvert pick. */
  formats: Record<FormatGroup["kind"], OutputFormat | null>;
  quality: number;
  metadata: MetadataMode;
};

/** Keeping all but the location suits an app that never sends files anywhere. */
export const DEFAULTS: Defaults = {
  formats: { video: null, audio: null, image: null },
  quality: 60,
  metadata: "noLocation",
};

/** Whether the editor offers `format` for an input of `kind`. */
export function offers(kind: MediaKind, format: OutputFormat): boolean {
  return groupsFor(kind).some((group) => group.formats.includes(format));
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

export type RecipeId = string;

export type Recipe = {
  id: RecipeId;
  /** Saved by the user from the editor (and removable). */
  custom?: boolean;
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

/**
 * `options` adapted to another file, or `null` when they don't fit it: a
 * format the editor wouldn't offer for it, sound from a silent file, or an
 * encoder that isn't installed. The trim belongs to one file and is dropped.
 */
export function fitOptions(
  info: MediaInfo,
  options: ConvertOptions,
  available: (f: OutputFormat) => boolean,
): ConvertOptions | null {
  if (!offers(info.kind, options.format) || !available(options.format)) return null;
  if (formatKind(options.format, info.kind) === "audio" && info.audioCodec === null) return null;
  const fitted = { ...options, jxl: { ...options.jxl }, trimStart: null, trimEnd: null };
  // Drop choices that would upscale or speed up this particular file.
  if (fitted.maxSize !== null && !sizeChoices(info).includes(fitted.maxSize)) {
    fitted.maxSize = null;
  }
  if (fitted.fps !== null && !fpsChoices(info).includes(fitted.fps)) fitted.fps = null;
  return fitted;
}

/** The user's default format for this file if it fits, otherwise CatConvert's pick. */
function preferredFormat(
  info: MediaInfo,
  available: (f: OutputFormat) => boolean,
  defaults: Defaults,
): OutputFormat {
  const chosen = info.kind === "unknown" ? null : defaults.formats[info.kind];
  if (chosen && chosen !== info.extension && available(chosen) && offers(info.kind, chosen)) {
    return chosen;
  }
  return defaultFormat(info, available);
}

/** Starting options for a freshly opened file, honouring a chosen recipe when it fits. */
export function initialOptions(
  info: MediaInfo,
  available: (f: OutputFormat) => boolean,
  recipe: Recipe | null,
  defaults: Defaults = DEFAULTS,
): ConvertOptions {
  const base = (format: OutputFormat): ConvertOptions => ({
    ...defaultOptions(format),
    quality: defaults.quality,
    metadata: defaults.metadata,
    jxl: { ...defaultOptions("jxl").jxl, lossless: LOSSLESS_BY_DEFAULT.has(info.extension ?? "") },
  });
  if (recipe?.accepts.includes(info.kind)) {
    const fitted = fitOptions(
      info,
      { ...base(recipe.options.format), ...recipe.options },
      available,
    );
    if (fitted) return fitted;
  }
  return base(preferredFormat(info, available, defaults));
}

const KIND_WORD = { video: "VIDEO", audio: "SOUND", image: "PICTURE", unknown: "ANY" } as const;
const KIND_TONE: Record<FormatGroup["kind"], Tone> = {
  video: "pink",
  audio: "yellow",
  image: "lavender",
};

/** One line describing what a recipe does, e.g. "720p · 30 fps · Balanced". */
export function recipeDetail(info: MediaInfo, options: ConvertOptions): string {
  const out = formatKind(options.format, info.kind);
  const parts: string[] = [];
  if (out !== "audio" && options.maxSize !== null) parts.push(sizeLabel(options.maxSize, out));
  if (out === "video" && options.fps !== null) parts.push(`${options.fps} fps`);
  const video = out === "video" && !isAnimatedImage(options.format, info.kind);
  if (video && !options.keepAudio) parts.push("no sound");
  parts.push(isLossless(options) ? "lossless" : qualityStep(options.quality).toLowerCase());
  if (options.metadata === "keep") parts.push("all metadata");
  if (options.metadata === "none") parts.push("no metadata");
  const text = parts.join(" · ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/**
 * A user recipe from the editor's current settings. It keeps the encoding
 * choices; the trim, turn and mirror belong to the file they were made for.
 */
export function recipeFrom(
  id: RecipeId,
  title: string,
  info: MediaInfo,
  options: ConvertOptions,
): Recipe {
  const out = formatKind(options.format, info.kind);
  const accepts: MediaKind[] = out === "audio" ? ["video", "audio"] : [info.kind];
  const { trimStart: _start, trimEnd: _end, rotate: _rotate, flip: _flip, ...kept } = options;
  return {
    id,
    title,
    custom: true,
    detail: recipeDetail(info, options),
    route: `${accepts.length > 1 ? "ANY" : KIND_WORD[info.kind]} → ${label(options.format)}`,
    tone: KIND_TONE[out],
    options: { ...kept, jxl: { ...kept.jxl } },
    accepts,
  };
}
