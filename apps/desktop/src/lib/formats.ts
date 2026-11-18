/**
 * What the UI knows about output formats, edit defaults and recipes. The
 * backend validates everything again; this only decides what to offer.
 */

import type {
  ConvertOptions,
  Destination,
  Direction,
  MediaInfo,
  MediaKind,
  MetadataMode,
  OutputFormat,
  Scaler,
  SequenceOptions,
  Upscale,
  VideoCodec,
} from "./api";
import type { CompareLayout } from "./compare";
import { aspectText, cropBox, cropForOtherFile, pictureSize, type Size, turnedSize } from "./crop";
import { formatBitrate } from "./format";

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
      return ["h264", "h265", "av1"];
    case "mov":
      return ["h264", "h265"];
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

export const CODEC_LABELS: Record<VideoCodec, string> = {
  h264: "H.264",
  h265: "H.265",
  vp9: "VP9",
  av1: "AV1",
};

/**
 * The streams of a video or sound file with their bitrates, for the file
 * info: "H264 12 Mbps · AAC 256 kbps", so the source can be compared with the
 * quality setting. Codec names stand alone where the file doesn't say, and
 * the whole file's bitrate is added when no stream's is known.
 */
export function describeStreams(info: MediaInfo): string | null {
  const stream = (codec: string | null, rate: number | null | undefined) =>
    codec
      ? [codec.toUpperCase(), rate ? formatBitrate(rate) : null].filter(Boolean).join(" ")
      : null;
  const parts =
    info.kind === "video"
      ? [stream(info.videoCodec, info.videoBitrate), stream(info.audioCodec, info.audioBitrate)]
      : info.kind === "audio"
        ? [stream(info.audioCodec, info.audioBitrate)]
        : [];
  const named = parts.filter((part): part is string => part !== null);
  if (named.length === 0) return null;
  if (!info.videoBitrate && !info.audioBitrate && info.bitrate) {
    named.push(`${formatBitrate(info.bitrate)} total`);
  }
  return named.join(" · ");
}

/** One line on what picking `codec` means. */
export function codecHint(codec: VideoCodec): string {
  switch (codec) {
    case "h265":
      return "Smaller files that keep HDR. Plays on Apple devices, recent PCs, TVs and phones.";
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
    !enlarges(info, options) &&
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
    upscale: null,
    scaler: "auto",
    fps: null,
    rotate: 0,
    flip: false,
    crop: null,
    quality: 60,
    keepAudio: true,
    videoCodec: null,
    videoBitrate: null,
    hardware: false,
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

/** Lowest and highest exact video bitrate in kbit/s, as `VIDEO_KBPS` in convert.rs. */
export const MIN_VIDEO_KBPS = 100;
export const MAX_VIDEO_KBPS = 200_000;

export function qualityStep(quality: number): (typeof QUALITY_STEPS)[number] {
  return QUALITY_STEPS[quality < 34 ? 0 : quality > 66 ? 2 : 1];
}

/** Where converted files go by default, chosen in Settings. */
export type OutputChoice = Destination["kind"];

/** What freshly opened files start with, set in Settings. */
export type Defaults = {
  /** Preferred output per input kind; `null` lets CatConvert pick. */
  formats: Record<FormatGroup["kind"], OutputFormat | null>;
  quality: number;
  metadata: MetadataMode;
  /** Where converted files go. */
  output: OutputChoice;
  /** The folder for `output: "folder"`, kept when another choice is made. */
  outputFolder: string | null;
  /** Sort the output folder into Videos, Pictures and Audio. */
  outputByKind: boolean;
  /** The user has agreed once that replacing moves originals to the Trash. */
  replaceConfirmed: boolean;
  /** The editor compares the original with the result (toggled there, with C). */
  compare: boolean;
  /** How the comparison is laid out on wide windows. */
  compareLayout: CompareLayout;
};

/** Keeping all but the location suits an app that never sends files anywhere. */
export const DEFAULTS: Defaults = {
  formats: { video: null, audio: null, image: null },
  quality: 60,
  metadata: "noLocation",
  output: "nextToInput",
  outputFolder: null,
  outputByKind: false,
  replaceConfirmed: false,
  compare: false,
  compareLayout: "wipe",
};

/**
 * The destination for `choice` (the default one unless given) with the
 * folder from Settings. A folder choice without a folder falls back to next
 * to the original.
 */
export function destinationFor(
  defaults: Defaults,
  choice: OutputChoice = defaults.output,
): Destination {
  if (choice === "folder") {
    return defaults.outputFolder
      ? { kind: "folder", path: defaults.outputFolder, byKind: defaults.outputByKind }
      : { kind: "nextToInput" };
  }
  return { kind: choice };
}

/** Whether `options` will replace the file they convert. */
export function replacesOriginal(options: Pick<ConvertOptions, "destination">): boolean {
  return options.destination?.kind === "replaceInput";
}

/** The last part of a folder path, e.g. `CatConvert`. */
export function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

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

// ---------------------------------------------------------------------------
// Upscaling
// ---------------------------------------------------------------------------

/** Factor presets for upscaling. */
export const UPSCALE_FACTORS = [2, 3, 4] as const;

/** Short sides to upscale to: 720p, 1080p, 1440p and 4K. */
export const UPSCALE_SIZES = [720, 1080, 1440, 2160] as const;

/** Bounds of a custom factor and target, as the core checks them. */
export const MAX_UPSCALE_FACTOR = 8;
export const MAX_UPSCALE_SIZE = 16384;

export const SCALERS: readonly { scaler: Scaler; label: string }[] = [
  // Lanczos is what "auto" picks for any enlargement, so it stands for both.
  { scaler: "auto", label: "Lanczos (sharp)" },
  { scaler: "spline", label: "Spline" },
  { scaler: "bicubic", label: "Bicubic (softer)" },
];

/**
 * Longest side the core writes in `format`: video encoders stop at 8192,
 * WebP at 16383, everything else at 16384. Bigger results are refused.
 */
export function maxLongSide(format: OutputFormat): number {
  if (videoCodecs(format).length > 0) return 8192;
  return format === "webp" ? 16383 : 16384;
}

/**
 * Area above which a result counts as huge and is worth a warning: 50
 * megapixels for a still, 4K for a video file, 1080p for an animation frame.
 */
export function hugePixels(format: OutputFormat, animated: boolean): number {
  if (videoCodecs(format).length > 0) return 3840 * 2160;
  return animated ? 1920 * 1080 : 50_000_000;
}

/** The short side `upscale` enlarges `short` to, or `null` when it doesn't enlarge it. */
export function upscaleTarget(short: number, upscale: Upscale | null | undefined): number | null {
  if (!upscale) return null;
  const target = "factor" in upscale ? Math.round(short * upscale.factor) : upscale.size;
  return target > short ? target : null;
}

/**
 * `size` after the upscale, then the cap on the short side (shrink only).
 * Video and animations keep even sides. Mirrors `output_dims` in convert.rs.
 */
export function scaledSize(
  size: Size,
  upscale: Upscale | null | undefined,
  cap: number | null,
  even: boolean,
): Size {
  const short = Math.min(size.width, size.height);
  const long = Math.max(size.width, size.height);
  const evened = (n: number) => (even ? Math.max(2, n & ~1) : n);
  let target = short;
  const up = upscaleTarget(short, upscale);
  if (up !== null) target = evened(up);
  if (cap !== null) target = Math.min(target, Math.max(2, cap) & ~1);
  if (target === short) {
    return even ? { width: evened(size.width), height: evened(size.height) } : size;
  }
  const scaledLong = Math.round((long * target) / short);
  const [width, height] = size.width >= size.height ? [scaledLong, target] : [target, scaledLong];
  return { width: evened(width), height: evened(height) };
}

/**
 * The size of what gets written, after the turn, crop, upscale and size cap.
 * An animation's default cap only applies when the picture isn't enlarged.
 */
export function outputSize(
  info: MediaInfo,
  options: Pick<ConvertOptions, "format" | "rotate" | "crop" | "maxSize" | "upscale">,
): Size | null {
  const size = pictureSize(info, options);
  if (!size) return null;
  const auto = enlarges(info, options)
    ? null
    : (autoLimits(options.format, info.kind)?.size ?? null);
  const even = formatKind(options.format, info.kind) === "video";
  return scaledSize(size, options.upscale, options.maxSize ?? auto, even);
}

/** Whether the options make the picture bigger than it is. */
export function enlarges(
  info: MediaInfo,
  options: Pick<ConvertOptions, "rotate" | "crop" | "upscale">,
): boolean {
  const short = shortSide(info, options);
  return short !== null && upscaleTarget(short, options.upscale) !== null;
}

/** Whether the result stays within what the core can write. Unknown sizes pass. */
export function fitsLimits(
  info: MediaInfo,
  options: Pick<ConvertOptions, "format" | "rotate" | "crop" | "maxSize" | "upscale">,
): boolean {
  const size = outputSize(info, options);
  return size === null || Math.max(size.width, size.height) <= maxLongSide(options.format);
}

/** Whether an enlarged result is big enough to warn about. */
export function isHuge(info: MediaInfo, options: ConvertOptions): boolean {
  if (!enlarges(info, options)) return false;
  const size = outputSize(info, options);
  const huge = hugePixels(options.format, isAnimatedImage(options.format, info.kind));
  return size !== null && size.width * size.height > huge;
}

/** Whether two upscale settings ask for the same thing. */
export function sameUpscale(a: Upscale | null | undefined, b: Upscale | null | undefined): boolean {
  if (!a || !b) return !a && !b;
  if ("factor" in a) return "factor" in b && a.factor === b.factor;
  return "size" in b && a.size === b.size;
}

/** "2×", or the target like a size choice: "1080p", "1080 px". */
export function upscaleLabel(upscale: Upscale, kind: MediaKind): string {
  return "factor" in upscale ? `${upscale.factor}×` : sizeLabel(upscale.size, kind);
}

/** What an upscale makes of a picture, and how big the format lets it get. */
type UpscaleBounds = {
  /** Short side before upscaling, if known. */
  short: number | null;
  /** The size written with `upscale` and no size cap. */
  resultFor: (upscale: Upscale) => Size | null;
  limit: number;
};

const fits = (bounds: UpscaleBounds, upscale: Upscale) => {
  const size = bounds.resultFor(upscale);
  return size === null || Math.max(size.width, size.height) <= bounds.limit;
};

/** Presets that enlarge the picture and fit the format: 2×, 3×, 4×, then bigger targets. */
function presetsWithin(bounds: UpscaleBounds): Upscale[] {
  const choices: Upscale[] = [
    ...UPSCALE_FACTORS.map((factor) => ({ factor })),
    ...UPSCALE_SIZES.map((size) => ({ size })),
  ];
  const { short } = bounds;
  if (short === null) return [];
  return choices.filter((u) => upscaleTarget(short, u) !== null && fits(bounds, u));
}

/** The biggest short side a custom upscale can reach, or `null` when the size is unknown. */
function biggestWithin(bounds: UpscaleBounds): number | null {
  const size = bounds.resultFor({ factor: 1 });
  if (!size || bounds.short === null) return null;
  const long = Math.max(size.width, size.height);
  let best = Math.min(MAX_UPSCALE_SIZE, Math.floor((bounds.limit * bounds.short) / long));
  // Rounding can push the long side one over; step back until it fits.
  while (best > bounds.short && !fits(bounds, { size: best })) best--;
  return best;
}

function fileBounds(info: MediaInfo, options: ConvertOptions): UpscaleBounds {
  return {
    short: shortSide(info, options),
    resultFor: (upscale) => outputSize(info, { ...options, upscale, maxSize: null }),
    limit: maxLongSide(options.format),
  };
}

function sequenceBounds(first: MediaInfo, options: ConvertOptions): UpscaleBounds {
  return {
    short: shortSide(first),
    resultFor: (upscale) => sequenceFrame(first, { ...options, upscale, maxSize: null }),
    limit: maxLongSide(options.format),
  };
}

/**
 * The upscale presets that enlarge this file without going past what the
 * core can write: 2×, 3×, 4×, then the targets above its short side.
 */
export const upscaleChoices = (info: MediaInfo, options: ConvertOptions) =>
  presetsWithin(fileBounds(info, options));

/** The biggest short side a custom upscale can reach on this file. */
export const maxUpscaleSize = (info: MediaInfo, options: ConvertOptions) =>
  biggestWithin(fileBounds(info, options));

/** {@link upscaleChoices} for an animation made from pictures, `first` setting the frame. */
export const sequenceUpscaleChoices = (first: MediaInfo, options: ConvertOptions) =>
  presetsWithin(sequenceBounds(first, options));

/** {@link maxUpscaleSize} for an animation made from pictures. */
export const maxSequenceUpscale = (first: MediaInfo, options: ConvertOptions) =>
  biggestWithin(sequenceBounds(first, options));

/** Whether an animation's frame stays within what the core can write. */
export function sequenceFits(first: MediaInfo, options: ConvertOptions): boolean {
  const frame = sequenceFrame(first, options);
  return frame === null || Math.max(frame.width, frame.height) <= maxLongSide(options.format);
}

/**
 * A stored upscale setting if it looks right, otherwise `null`. Settings
 * saved before upscaling existed have none.
 */
export function upscaleFrom(stored: unknown): Upscale | null {
  if (!stored || typeof stored !== "object") return null;
  const { factor, size } = stored as { factor?: unknown; size?: unknown };
  if (typeof factor === "number" && factor >= 1 && factor <= MAX_UPSCALE_FACTOR) return { factor };
  if (typeof size === "number" && Number.isInteger(size) && size >= 2 && size <= MAX_UPSCALE_SIZE) {
    return { size };
  }
  return null;
}

/** A stored scaler if it is one, otherwise `"auto"`. */
export function scalerFrom(stored: unknown): Scaler {
  return stored === "lanczos" || stored === "spline" || stored === "bicubic" ? stored : "auto";
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
  /**
   * A command edited by hand, with placeholders for the paths, run instead
   * of the generated one for files that keep the recipe's format.
   */
  command?: string;
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
  // An upscale applies to any file (a target it already reaches does
  // nothing), unless it would be too big to write. One that does nothing
  // would still lift an animation's default size cap, so it goes too.
  const upscale = upscaleFrom(fitted.upscale);
  const lifts = !enlarges(info, { ...fitted, upscale }) && autoLimits(fitted.format, info.kind);
  fitted.upscale = upscale && !lifts && fitsLimits(info, { ...fitted, upscale }) ? upscale : null;
  fitted.scaler = scalerFrom(fitted.scaler);
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
    destination: destinationFor(defaults),
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
  if (out !== "audio" && options.upscale) {
    parts.push(
      "factor" in options.upscale
        ? `${options.upscale.factor}× upscale`
        : `upscaled to ${sizeLabel(options.upscale.size, out)}`,
    );
  }
  if (out !== "audio" && options.maxSize !== null) parts.push(sizeLabel(options.maxSize, out));
  if (out === "video" && options.fps !== null) parts.push(`${options.fps} fps`);
  if (isAnimatedImage(options.format, info.kind)) {
    if (options.speed !== 1) parts.push(speedLabel(options.speed));
    if (options.direction !== "forward") parts.push(options.direction);
  }
  const video = out === "video" && !isAnimatedImage(options.format, info.kind);
  const codec = codecOf(options);
  if (video && codec && codec !== videoCodecs(options.format)[0]) parts.push(CODEC_LABELS[codec]);
  if (video && codec === "h265" && options.hardware) parts.push("hardware");
  if (video && !options.keepAudio) parts.push("no sound");
  const exact = video && codec !== null ? (options.videoBitrate ?? null) : null;
  if (exact !== null) parts.push(formatBitrate(exact * 1000));
  else parts.push(isLossless(options) ? "lossless" : qualityStep(options.quality).toLowerCase());
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
  command: string | null = null,
): Recipe {
  const out = formatKind(options.format, info.kind);
  const accepts: MediaKind[] = out === "audio" ? ["video", "audio"] : [info.kind];
  // Where results go is a setting of its own, not part of a recipe.
  const {
    trimStart: _start,
    trimEnd: _end,
    rotate: _rotate,
    flip: _flip,
    destination: _destination,
    ...kept
  } = options;
  const crop = out === "audio" ? null : cropForOtherFile(options.crop);
  return {
    id,
    title,
    custom: true,
    detail: command === null ? recipeDetail(info, { ...options, crop }) : "Custom command",
    route: `${accepts.length > 1 ? "ANY" : KIND_WORD[info.kind]} → ${label(options.format)}`,
    tone: KIND_TONE[out],
    options: { ...kept, crop, jxl: { ...kept.jxl }, sound: { ...kept.sound } },
    accepts,
    ...(command === null ? {} : { command }),
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

/**
 * The animation's frame: the first picture's size, upscaled and capped, with
 * even sides. The default cap only applies when nothing asks to upscale.
 */
export function sequenceFrame(
  first: MediaInfo,
  options: ConvertOptions,
): { width: number; height: number } | null {
  if (!first.width || !first.height) return null;
  const cap = options.maxSize ?? (options.upscale ? null : sequenceAutoSize(options.format));
  const { width, height } = scaledSize(
    { width: first.width, height: first.height },
    options.upscale,
    cap,
    true,
  );
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
    // One file can't take the place of several pictures.
    destination: destinationFor(
      defaults,
      defaults.output === "replaceInput" ? "nextToInput" : defaults.output,
    ),
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
