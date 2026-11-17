/**
 * What the UI knows about output formats, edit defaults and recipes. The
 * backend validates everything again; this only decides what to offer.
 */
import type {
  ConvertOptions,
  Direction,
  MediaInfo,
  MediaKind,
  MetadataMode,
  OutputFormat,
  SequenceOptions,
  VideoCodec,
} from "./api";
import { aspectText, cropBox, cropForOtherFile, pictureSize, turnedSize } from "./crop";

export type Tone = "pink" | "green" | "yellow" | "lavender";

export type FormatGroup = {
  kind: Exclude<MediaKind, "unknown">;
  label: string;
  tone: Tone;
  formats: readonly OutputFormat[];
};

/** WebP and JPEG XL appear twice: as animations for videos, as stills for pictures. */
export const FORMAT_GROUPS: readonly FormatGroup[] = [
  {
    kind: "video",
    label: "Video",
    tone: "pink",
    formats: ["mp4", "webm", "mov", "mkv", "gif", "webp", "jxl"],
  },
  { kind: "audio", label: "Sound", tone: "yellow", formats: ["mp3", "m4a", "wav", "flac", "ogg"] },
  {
    kind: "image",
    label: "Picture",
    tone: "lavender",
    formats: ["png", "jpg", "webp", "avif", "jxl"],
  },
];

/** Formats that are a still or an animation, depending on the input. */
const STILL_OR_ANIMATED: ReadonlySet<OutputFormat> = new Set(["webp", "jxl"]);

/** What writing `format` from an `input` file makes. WebP and JPEG XL move when the input does. */
export function formatKind(format: OutputFormat, input: MediaKind): FormatGroup["kind"] {
  if (STILL_OR_ANIMATED.has(format)) return input === "video" ? "video" : "image";
  return FORMAT_GROUPS.find((group) => group.formats.includes(format))?.kind ?? "video";
}

/** Formats that become animated images rather than video files (no sound). */
export function isAnimatedImage(format: OutputFormat, input: MediaKind): boolean {
  return format === "gif" || (STILL_OR_ANIMATED.has(format) && input === "video");
}

/**
 * Size and frame-rate caps the backend applies to animations when the user
 * keeps "Auto". Mirrors GIF_MAX_SIZE/GIF_FPS and ANIMATED_* in convert.rs.
 */
export function autoLimits(
  format: OutputFormat,
  input: MediaKind,
): { size: number; fps: number } | null {
  if (format === "gif") return { size: 480, fps: 15 };
  if (input !== "video") return null;
  if (format === "webp") return { size: 480, fps: 24 };
  if (format === "jxl") return { size: 720, fps: 30 };
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

/** Speed choices for animations. */
export const SPEEDS = [0.5, 1, 2, 4] as const;

export const DIRECTIONS: readonly { direction: Direction; label: string }[] = [
  { direction: "forward", label: "Forward" },
  { direction: "reverse", label: "Reverse" },
  { direction: "boomerang", label: "Boomerang" },
];

export const speedLabel = (speed: number) => `${speed}×`;

/** Whether the speed or direction change how this output plays. */
export function changesPlayback(info: MediaInfo, options: ConvertOptions): boolean {
  return (
    isAnimatedImage(options.format, info.kind) &&
    (options.speed !== 1 || options.direction !== "forward")
  );
}

/**
 * Length of the output in seconds: the kept range, and for animations its
 * speed and direction (a boomerang plays it twice). `null` if unknown.
 */
export function outputSeconds(info: MediaInfo, options: ConvertOptions): number | null {
  const end = options.trimEnd ?? info.durationSecs;
  if (end === null) return null;
  const kept = Math.max(0, end - (options.trimStart ?? 0));
  if (!isAnimatedImage(options.format, info.kind)) return kept;
  return (kept / options.speed) * (options.direction === "boomerang" ? 2 : 1);
}

/** Formats whose quality setting changes nothing. */
export const LOSSLESS: ReadonlySet<OutputFormat> = new Set(["wav", "flac", "png"]);

/** Whether these options keep every pixel/sample, so quality changes nothing. */
export function isLossless(options: ConvertOptions): boolean {
  return LOSSLESS.has(options.format) || (options.format === "jxl" && options.jxl.lossless);
}

export const label = (format: OutputFormat) => format.toUpperCase();

/**
 * The video codecs a format can carry, its usual one first; empty for formats
 * without a choice. Mirrors `OutputFormat::video_codecs` in convert.rs.
 */
export function videoCodecs(format: OutputFormat): readonly VideoCodec[] {
  switch (format) {
    case "mp4":
    case "mkv":
      return ["h264", "av1"];
    case "mov":
      return ["h264"];
    case "webm":
      return ["vp9", "av1"];
    default:
      return [];
  }
}

/** The codec the picture is encoded with, or `null` for formats without a choice. */
export function codecOf(options: Pick<ConvertOptions, "format" | "videoCodec">): VideoCodec | null {
  const codecs = videoCodecs(options.format);
  if (options.videoCodec && codecs.includes(options.videoCodec)) return options.videoCodec;
  return codecs[0] ?? null;
}

/**
 * The codec choice to keep when switching to `format`: the same codec if the
 * format can hold it (AV1 MP4 → AV1 WebM), otherwise the format's usual one.
 */
export function codecFor(format: OutputFormat, codec: VideoCodec | null): VideoCodec | null {
  return codec && videoCodecs(format).includes(codec) ? codec : null;
}

export const CODEC_LABELS: Record<VideoCodec, string> = { h264: "H.264", vp9: "VP9", av1: "AV1" };

/** One line on what picking `codec` means. */
export function codecHint(codec: VideoCodec): string {
  switch (codec) {
    case "av1":
      return "Smaller files that keep HDR, but slower to make. Plays on recent devices and browsers.";
    case "vp9":
      return "Plays in every browser.";
    default:
      return "Plays everywhere, quick to make.";
  }
}

/** Tile colour for each kind of file. */
export const KIND_TONE: Record<MediaKind, Tone> = {
  video: "pink",
  audio: "yellow",
  image: "lavender",
  unknown: "green",
};

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
 * crop, resize, new rate, speed or direction. Dropping metadata doesn't
 * count; the backend cleans the JPEG before libjxl repacks it (see scrub.rs).
 */
export function isUntouched(info: MediaInfo, options: ConvertOptions): boolean {
  const untrimmed =
    info.kind === "image" || (options.trimStart === null && options.trimEnd === null);
  return (
    untrimmed &&
    !changesPlayback(info, options) &&
    options.rotate === 0 &&
    !options.flip &&
    !hasCrop(info, options) &&
    (options.maxSize === null || !sizeChoices(info, options).includes(options.maxSize)) &&
    (options.fps === null || !fpsChoices(info).includes(options.fps))
  );
}

/** A JPEG XL that holds a recompressed JPEG, turned back into that exact JPEG. */
/** Whether the crop cuts anything off this file. Unknown sizes count as cut. */
export function hasCrop(info: MediaInfo, options: Pick<ConvertOptions, "rotate" | "crop">) {
  if (!options.crop) return false;
  const size = turnedSize(info, options.rotate);
  return size === null || cropBox(size, options.crop) !== null;
}

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
    crop: null,
    quality: 60,
    keepAudio: true,
    videoCodec: null,
    jxl: { lossless: false, effort: 7, progressive: false },
    sound: { normalize: false, fadeIn: 0, fadeOut: 0 },
    metadata: "noLocation",
    speed: 1,
    direction: "forward",
  };
}

/** Fade lengths offered in the Sound section, in seconds (0 is off). */
export const FADE_CHOICES = [0, 1, 3] as const;

/** Whether the output carries sound, so the Sound settings apply. */
export function outputHasSound(info: MediaInfo, options: ConvertOptions): boolean {
  if (info.audioCodec === null) return false;
  const out = formatKind(options.format, info.kind);
  if (out === "audio") return true;
  return out === "video" && !isAnimatedImage(options.format, info.kind) && options.keepAudio;
}

/**
 * How long the fades run over a kept range of `length` seconds, clipped to it
 * as the planner does. A fade-out needs a known length.
 */
export function fadeSpans(
  sound: ConvertOptions["sound"],
  length: number | null,
): { fadeIn: number; fadeOut: number } {
  const clip = (secs: number) => Math.max(0, length === null ? secs : Math.min(secs, length));
  return { fadeIn: clip(sound.fadeIn), fadeOut: length === null ? 0 : clip(sound.fadeOut) };
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

type Framing = Pick<ConvertOptions, "rotate" | "crop">;

/** Short side of the picture after rotation metadata and the crop, if known. */
export function shortSide(info: MediaInfo, framing?: Framing): number | null {
  const size = pictureSize(info, framing ?? { rotate: 0, crop: null });
  return size ? Math.min(size.width, size.height) : null;
}

const SIZES = [2160, 1440, 1080, 720, 480, 360, 240] as const;

/** Downscale choices smaller than the (cropped) source. */
export function sizeChoices(info: MediaInfo, framing?: Framing): number[] {
  const short = shortSide(info, framing);
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
 * encoder that isn't installed. The trim and a hand-drawn crop belong to one
 * file and are dropped; a crop shape is kept and centred.
 */
export function fitOptions(
  info: MediaInfo,
  options: ConvertOptions,
  available: (f: OutputFormat) => boolean,
): ConvertOptions | null {
  if (!offers(info.kind, options.format) || !available(options.format)) return null;
  if (formatKind(options.format, info.kind) === "audio" && info.audioCodec === null) return null;
  const fitted = {
    ...options,
    videoCodec: codecFor(options.format, options.videoCodec ?? null),
    jxl: { ...options.jxl },
    sound: { ...options.sound },
    trimStart: null,
    trimEnd: null,
    crop: cropForOtherFile(options.crop),
  };
  // Drop choices that would upscale or speed up this particular file.
  if (fitted.maxSize !== null && !sizeChoices(info, fitted).includes(fitted.maxSize)) {
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

/** One line describing what a recipe does, e.g. "720p · 30 fps · Balanced". */
export function recipeDetail(info: MediaInfo, options: ConvertOptions): string {
  const out = formatKind(options.format, info.kind);
  const parts: string[] = [];
  if (out !== "audio" && options.crop?.aspect)
    parts.push(`${aspectText(options.crop.aspect)} crop`);
  if (out !== "audio" && options.maxSize !== null) parts.push(sizeLabel(options.maxSize, out));
  if (out === "video" && options.fps !== null) parts.push(`${options.fps} fps`);
  if (isAnimatedImage(options.format, info.kind)) {
    if (options.speed !== 1) parts.push(speedLabel(options.speed));
    if (options.direction !== "forward") parts.push(options.direction);
  }
  const video = out === "video" && !isAnimatedImage(options.format, info.kind);
  const codec = codecOf(options);
  if (video && codec && codec !== videoCodecs(options.format)[0]) parts.push(CODEC_LABELS[codec]);
  if (video && !options.keepAudio) parts.push("no sound");
  parts.push(isLossless(options) ? "lossless" : qualityStep(options.quality).toLowerCase());
  if (outputHasSound(info, options)) {
    if (options.sound.normalize) parts.push("even loudness");
    if (options.sound.fadeIn > 0 || options.sound.fadeOut > 0) parts.push("fades");
  }
  if (options.metadata === "keep") parts.push("all metadata");
  if (options.metadata === "none") parts.push("no metadata");
  const text = parts.join(" · ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/**
 * A user recipe from the editor's current settings. It keeps the encoding
 * choices and the crop's shape; the trim, turn, mirror and where the crop
 * sits belong to the file they were made for.
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
  const crop = out === "audio" ? null : cropForOtherFile(options.crop);
  return {
    id,
    title,
    custom: true,
    detail: recipeDetail(info, { ...options, crop }),
    route: `${accepts.length > 1 ? "ANY" : KIND_WORD[info.kind]} → ${label(options.format)}`,
    tone: KIND_TONE[out],
    options: { ...kept, crop, jxl: { ...kept.jxl }, sound: { ...kept.sound } },
    accepts,
  };
}

// ---------------------------------------------------------------------------
// Pictures → animation
// ---------------------------------------------------------------------------

/** What pictures can become, in the order the sequence editor offers them. */
export const SEQUENCE_FORMATS: readonly OutputFormat[] = ["gif", "webp", "jxl", "mp4"];

/** Frame-time choices, in seconds. */
export const FRAME_TIMES = [0.1, 0.25, 0.5, 1] as const;

export const DEFAULT_SEQUENCE: SequenceOptions = {
  frameSecs: 0.5,
  fit: "pad",
};

/** Extensions of still pictures. GIF and APNG may move, so they don't count. */
const PICTURE_EXTENSIONS = new Set([
  ...["jpg", "jpeg", "png", "webp", "avif", "heic", "heif", "hif", "tif", "tiff", "bmp", "jxl"],
]);

/** Whether a dropped path looks like a still picture, judging by its name. */
export function looksLikePicture(path: string): boolean {
  const dot = path.lastIndexOf(".");
  return dot > 0 && PICTURE_EXTENSIONS.has(path.slice(dot + 1).toLowerCase());
}

const NATURAL = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Natural filename order: `cat 2` before `cat 10`. */
export function naturalOrder(a: MediaInfo, b: MediaInfo): number {
  return NATURAL.compare(a.fileName, b.fileName) || NATURAL.compare(a.path, b.path);
}

/**
 * Default short-side cap of an animation made from pictures, when "Auto" is
 * kept. Mirrors `frame_size` in sequence.rs.
 */
export function sequenceAutoSize(format: OutputFormat): number | null {
  if (format === "gif" || format === "webp") return 480;
  if (format === "jxl") return 720;
  return null;
}

/** The animation's frame: the first picture's size, capped, with even sides. */
export function sequenceFrame(
  first: MediaInfo,
  options: ConvertOptions,
): { width: number; height: number } | null {
  if (!first.width || !first.height) return null;
  const [w, h] = [first.width, first.height];
  const cap = options.maxSize ?? sequenceAutoSize(options.format);
  const target = cap === null ? null : Math.max(2, cap) & ~1;
  let [width, height] = [w, h];
  if (target !== null && target < Math.min(w, h)) {
    const long = Math.round((Math.max(w, h) * target) / Math.min(w, h));
    [width, height] = w >= h ? [long, target] : [target, long];
  }
  const even = (n: number) => Math.max(2, n & ~1);
  [width, height] = [even(width), even(height)];
  return options.rotate % 180 === 90 ? { width: height, height: width } : { width, height };
}

/** The order pictures show in. Mirrors `Direction::order` in sequence.rs. */
export function playbackOrder(count: number, direction: Direction): number[] {
  const forward = Array.from({ length: count }, (_, i) => i);
  if (direction === "reverse") return forward.reverse();
  if (direction === "boomerang") return [...forward, ...forward.slice(1, -1).reverse()];
  return forward;
}

/** Starting format for an animation: the first offered one that is installed. */
export function sequenceFormat(animated: (f: OutputFormat) => boolean): OutputFormat {
  return SEQUENCE_FORMATS.find(animated) ?? "gif";
}

/** Starting options for an animation made from pictures. */
export function sequenceOptions(
  animated: (f: OutputFormat) => boolean,
  defaults: Defaults = DEFAULTS,
): ConvertOptions {
  return {
    ...defaultOptions(sequenceFormat(animated)),
    quality: defaults.quality,
    // An animation never carries one picture's tags; the backend strips them.
    metadata: "none",
  };
}

/**
 * The animation's file name without extension: what the pictures' names
 * share (`cat 1`, `cat 2` → `cat`), else their folder. Mirrors
 * `sequence_name` in sequence.rs; the backend adds ` 2` and so on if taken.
 */
export function sequenceName(pictures: readonly MediaInfo[]): string {
  const stem = (info: MediaInfo) => {
    const dot = info.fileName.lastIndexOf(".");
    return dot > 0 ? info.fileName.slice(0, dot) : info.fileName;
  };
  const stems = pictures.map(stem);
  let prefix = [...(stems[0] ?? "")];
  for (const other of stems.slice(1)) {
    const chars = [...other];
    let shared = 0;
    while (shared < prefix.length && prefix[shared] === chars[shared]) shared++;
    prefix = prefix.slice(0, shared);
  }
  const trimmed = prefix.join("").replace(/[\d\s_\-.#(]+$/u, "");
  if ((trimmed.match(/[\p{L}\p{N}]/gu) ?? []).length >= 2) return trimmed;
  const parts = (pictures[0]?.path ?? "").split(/[\\/]/);
  const folder = (parts[parts.length - 2] ?? "").replace(/^\.+/, "");
  return folder || "animation";
}
