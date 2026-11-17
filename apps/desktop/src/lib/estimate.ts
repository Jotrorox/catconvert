/**
 * A rough guess of the output size, shown before converting so the quality
 * slider means something. The encoder settings mirror `ffmpeg_step`,
 * `jxl_distance` and `output_dims` in crates/core/src/convert.rs; the
 * bits-per-pixel curves are typical figures for each encoder at those
 * settings, scaled by how busy the source looks (its own bitrate against what
 * is usual for its codec). Content still matters more than any of this, so
 * the UI calls it "about".
 */
import type { ConvertOptions, MediaInfo } from "./api";
import { pictureSize } from "./crop";
import { formatBytes } from "./format";
import { autoLimits, formatKind, isAnimatedImage, repacksJpeg, restoresJpeg } from "./formats";

/** Same as `lerp` in convert.rs: quality 0–100 onto `worst..=best`, rounded. */
function lerp(quality: number, worst: number, best: number): number {
  return Math.round(worst + ((best - worst) * quality) / 100);
}

/** Same as `tiered` in convert.rs. */
function tiered<T>(quality: number, [low, mid, high]: readonly [T, T, T]): T {
  return quality < 34 ? low : quality < 67 ? mid : high;
}

const kbps = (rate: number) => rate * 1000;

/** CD audio; the usual size of decoded sound. */
const PCM_RATE = kbps(1411.2);

/** LAME's average bitrate for -q:a 0 (V0) to 7 (V7). */
const MP3_VBR = [245, 225, 190, 175, 165, 130, 115, 100].map(kbps);

const FULL_HD = 1920 * 1080;

/** Bits per second of the sound track the encoder writes. */
function audioRate(info: MediaInfo, options: ConvertOptions, sourceRate: number | null): number {
  const q = options.quality;
  const pcm = info.audioCodec?.startsWith("pcm") ?? false;
  switch (options.format) {
    case "mp3":
      return MP3_VBR[lerp(q, 7, 0)] ?? kbps(165);
    case "m4a":
      return kbps(tiered(q, [128, 192, 256] as const));
    case "ogg":
    case "webm":
      return kbps(tiered(q, [64, 96, 160] as const));
    case "wav":
      return pcm && sourceRate !== null ? sourceRate : PCM_RATE;
    case "flac":
      if (info.audioCodec === "flac" && sourceRate !== null) return sourceRate;
      return 0.6 * (pcm && sourceRate !== null ? sourceRate : PCM_RATE);
    default:
      // MP4, MOV and MKV carry AAC.
      return kbps(tiered(q, [96, 128, 192] as const));
  }
}

/** Picture area after the crop and downscaling, like `output_dims`. */
function outputPixels(
  info: MediaInfo,
  options: ConvertOptions,
  maxSize: number | null,
): number | null {
  const size = pictureSize(info, options);
  if (!size) return null;
  const short = Math.min(size.width, size.height);
  const long = Math.max(size.width, size.height);
  const target = maxSize === null ? null : Math.max(2, maxSize) & ~1;
  if (target === null || target >= short) return size.width * size.height;
  return target * Math.round((long * target) / short);
}

/** Typical video bits per pixel per frame for a source codec at 1080p. */
const TYPICAL_VIDEO_BPP: Record<string, number> = {
  h264: 0.15,
  hevc: 0.09,
  vp9: 0.09,
  av1: 0.07,
  mpeg4: 0.2,
};

/** Typical still-image bits per pixel for a source format. */
const TYPICAL_IMAGE_BPP: Record<string, number> = {
  jpg: 2.5,
  jpeg: 2.5,
  png: 12,
  webp: 1.5,
  avif: 1,
  heic: 1.2,
  heif: 1.2,
  jxl: 1.5,
};

/** Bigger frames need fewer bits per pixel for the same look. */
const resolutionFactor = (pixels: number) => (pixels / FULL_HD) ** -0.25;

/**
 * How much detail and motion the source holds compared to a typical file of
 * its kind: above 1 for busy footage or photos, below 1 for screen recordings
 * and flat graphics. Damped and bounded, since a source's bitrate also
 * reflects how it was encoded.
 */
function complexity(info: MediaInfo, sourceAudioRate: number): number {
  const clamp = (ratio: number) => Math.min(2.5, Math.max(0.35, Math.sqrt(ratio)));
  if (!info.width || !info.height || info.sizeBytes <= 0) return 1;
  const pixels = info.width * info.height;
  if (info.kind === "image") {
    const typical = info.reconstructsJpeg ? 2 : TYPICAL_IMAGE_BPP[info.extension ?? ""];
    if (typical === undefined) return 1;
    return clamp((info.sizeBytes * 8) / pixels / typical);
  }
  if (info.kind !== "video" || !info.durationSecs || !info.fps) return 1;
  const typical = TYPICAL_VIDEO_BPP[info.videoCodec ?? ""];
  if (typical === undefined) return 1;
  const videoRate = (info.sizeBytes * 8) / info.durationSecs - sourceAudioRate;
  if (videoRate <= 0) return 1;
  return clamp(videoRate / (pixels * info.fps) / (typical * resolutionFactor(pixels)));
}

/** Bits per pixel of a JPEG XL frame at the Butteraugli distance for these options. */
function jxlBpp(options: ConvertOptions): number {
  const effort = options.jxl.effort <= 3 ? 1.1 : options.jxl.effort >= 9 ? 0.95 : 1;
  if (options.jxl.lossless) return 7.5 * effort;
  const distance = 3 - (2.5 * options.quality) / 100;
  return 1.2 * distance ** -0.7 * effort;
}

/** Bits per pixel of a still image in `format`. */
function imageBpp(options: ConvertOptions): number {
  const q = options.quality;
  switch (options.format) {
    case "jpg":
      return 7 / lerp(q, 20, 2) ** 0.85;
    case "webp":
      return 0.17 * Math.exp(0.03 * lerp(q, 40, 95));
    case "avif":
      return 0.7 * 2 ** ((30 - lerp(q, 45, 20)) / 9);
    case "jxl":
      return jxlBpp(options);
    default:
      // PNG of a photo.
      return 12;
  }
}

/** Estimated output size in bytes, or `null` when too little is known to guess. */
export function estimateBytes(info: MediaInfo, options: ConvertOptions): number | null {
  if (info.kind === "unknown") return null;
  const out = formatKind(options.format, info.kind);
  const q = options.quality;
  const sourceRate =
    info.durationSecs && info.sizeBytes > 0 ? (info.sizeBytes * 8) / info.durationSecs : null;

  // Exact round trips of a JPEG through JPEG XL.
  if (repacksJpeg(info, options)) return info.sizeBytes * 0.8;
  if (restoresJpeg(info, options)) return info.sizeBytes / 0.8;

  if (out === "image") {
    // A picture, or one frame of a video.
    const pixels = outputPixels(info, options, options.maxSize);
    if (pixels === null) return null;
    let bpp = imageBpp(options);
    if (info.kind === "image") {
      if (options.format === "png" && info.extension === "png") {
        bpp = (info.sizeBytes * 8) / ((info.width ?? 1) * (info.height ?? 1));
      } else {
        bpp *= complexity(info, 0);
      }
    }
    return (pixels * bpp) / 8;
  }

  const start = options.trimStart ?? 0;
  const end = options.trimEnd ?? info.durationSecs;
  if (end === null) return null;
  const seconds = Math.max(0, end - start);

  if (out === "audio") {
    if (info.audioCodec === null) return null;
    // Only an audio file's bitrate is the sound's own.
    return (audioRate(info, options, info.kind === "audio" ? sourceRate : null) * seconds) / 8;
  }

  // Video, GIF or animated JPEG XL.
  const sourceAudio = info.audioCodec === null ? 0 : kbps(128);
  const limits = autoLimits(options.format, info.kind);
  const pixels = outputPixels(info, options, options.maxSize ?? limits?.size ?? null);
  if (pixels === null) return null;
  const sourceFps = info.fps ?? 30;
  const fps = Math.min(sourceFps, options.fps ?? limits?.fps ?? sourceFps);
  const busy = complexity(info, sourceAudio);
  const frames = pixels * fps * seconds;

  if (isAnimatedImage(options.format, info.kind)) {
    if (options.format === "gif") {
      // LZW over a dithered palette: more colours, more bits.
      const colors = lerp(q, 32, 256);
      return (frames * 0.35 * Math.log2(colors) * busy) / 8;
    }
    return (frames * jxlBpp(options) * busy) / 8;
  }

  // Each step of CRF changes the size by a fixed ratio. x264 doubles about
  // every 6, VP9 about every 8; VP9 needs roughly 30% fewer bits.
  const bpp =
    options.format === "webm"
      ? 0.056 * 2 ** ((31 - lerp(q, 48, 28)) / 8)
      : 0.08 * 2 ** ((23 - lerp(q, 34, 18)) / 6);
  const videoBits = frames * bpp * resolutionFactor(pixels) * (fps / 30) ** -0.3 * busy;
  const keepAudio = options.keepAudio && info.audioCodec !== null;
  const audioBits = keepAudio ? audioRate(info, options, null) * seconds : 0;
  return (videoBits + audioBits) / 8;
}

/** Round to two significant digits so the number doesn't promise precision. */
function roughly(bytes: number): number {
  if (bytes <= 0) return 0;
  const scale = 10 ** (Math.floor(Math.log10(bytes)) - 1);
  return Math.round(bytes / scale) * scale;
}

/** One line for the editor, e.g. "About 12 MB · 75% smaller". */
export function describeEstimate(info: MediaInfo, options: ConvertOptions): string | null {
  const bytes = estimateBytes(info, options);
  if (bytes === null || !Number.isFinite(bytes)) return null;
  const size = `About ${formatBytes(roughly(bytes)).replace(".0 ", " ")}`;
  if (info.sizeBytes <= 0) return size;
  const ratio = bytes / info.sizeBytes;
  if (ratio >= 2) return `${size} · ${Math.round(ratio)}× bigger`;
  if (ratio > 1.05) return `${size} · ${Math.max(5, Math.round((ratio - 1) * 20) * 5)}% bigger`;
  if (ratio <= 0.1) return `${size} · ${Math.round(1 / ratio)}× smaller`;
  if (ratio < 0.95) return `${size} · ${Math.max(5, Math.round((1 - ratio) * 20) * 5)}% smaller`;
  return `${size} · about the same size`;
}
