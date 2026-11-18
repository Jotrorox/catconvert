/**
 * Typed bridge to the Tauri backend.
 *
 * Every `invoke` and `listen` call in the app goes through this module, so the
 * command/event contract with `apps/desktop/src-tauri/src/lib.rs` lives in one
 * place. The types mirror the serde output of `catconvert-core` (camelCase).
 */
import { getVersion } from "@tauri-apps/api/app";
import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { message, open, save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { check, type Update } from "@tauri-apps/plugin-updater";

export type { UnlistenFn };

// ---------------------------------------------------------------------------
// Types shared with catconvert-core
// ---------------------------------------------------------------------------

export type MediaKind = "image" | "video" | "audio" | "unknown";

export type MediaInfo = {
  path: string;
  fileName: string;
  extension: string | null;
  sizeBytes: number;
  kind: MediaKind;
  durationSecs: number | null;
  /** Display size, rotation metadata already applied. */
  width: number | null;
  height: number | null;
  fps: number | null;
  videoCodec: string | null;
  audioCodec: string | null;
  /** Sample rate of the sound in Hz. */
  sampleRate: number | null;
  /** Channel count of the sound, e.g. 2 for stereo. */
  audioChannels?: number | null;
  /** Average bitrate of the whole file in bit/s. */
  bitrate?: number | null;
  /** Average bitrate of the picture in bit/s, when the file says or it can be worked out. */
  videoBitrate?: number | null;
  /** Average bitrate of the sound in bit/s, when the file says or it can be worked out. */
  audioBitrate?: number | null;
  /** JPEG XL only: a losslessly recompressed JPEG whose original can be restored. */
  reconstructsJpeg: boolean;
  /** HDR video (PQ or HLG, as iPhones record); pictures made from it are tone-mapped. */
  hdr: boolean;
  /** GPS in a photo's EXIF or XMP, or a location tag in a video or song. */
  hasLocation: boolean;
  /** Container tags holding the location, cleared when everything else is kept. */
  locationTags: string[];
  /** HEIF only: the stream group ffmpeg stitches a tiled photo from. */
  tileGrid: number | null;
  /** EXIF orientation (2–8) ffmpeg applies when decoding; `width`/`height` already include it. */
  orientation?: number | null;
};

export type OutputFormat =
  | "mp4"
  | "webm"
  | "mov"
  | "mkv"
  | "gif"
  | "mp3"
  | "m4a"
  | "wav"
  | "flac"
  | "ogg"
  | "png"
  | "jpg"
  | "webp"
  | "avif"
  | "jxl";

/** How the picture of an MP4, MOV, MKV or WebM file is compressed. */
export type VideoCodec = "h264" | "h265" | "vp9" | "av1";

export type Rotation = 0 | 90 | 180 | 270;

/** What to carry over besides the picture and sound. */
export type MetadataMode = "keep" | "noLocation" | "none";

/** Which way an animation plays; a boomerang plays forward, then back. */
export type Direction = "forward" | "reverse" | "boomerang";

/** Width to height, e.g. `[9, 16]`. */
export type Aspect = [number, number];

/** A rectangle in fractions (0 to 1) of the picture's width and height. */
export type CropArea = { x: number; y: number; width: number; height: number };

/** Part of the picture to keep, measured after the turn and mirror. */
export type Crop = {
  /** `null` for a free-form crop. */
  aspect: Aspect | null;
  /** `null` keeps the largest centred area with `aspect`, on any file. */
  area: CropArea | null;
};

/** Settings that only matter when writing JPEG XL. */
export type JxlOptions = {
  /** Mathematically lossless; an unedited JPEG is recompressed and can be restored exactly. */
  lossless: boolean;
  /** 1 (fast) to 10 (slow, smallest). */
  effort: number;
  /** Sharp preview early while the file loads. */
  progressive: boolean;
};

/** Clean-up for the sound; ignored when the output has none. */
export type SoundOptions = {
  /** Even out loudness (to -16 LUFS). */
  normalize: boolean;
  /** Seconds to fade in from silence (and black, for video); 0 is off. */
  fadeIn: number;
  /** Seconds to fade out to silence (and black, for video); 0 is off. */
  fadeOut: number;
};

/** Which JPEG XL code paths are installed. */
export type JxlSupport = {
  cjxl: boolean;
  djxl: boolean;
  ffmpegEncoder: boolean;
  ffmpegAnimEncoder: boolean;
  ffmpegDecoder: boolean;
  ffmpegAnimDecoder: boolean;
};

/**
 * Where a conversion's result goes. `folder` with `byKind` sorts into its
 * Videos, Pictures and Audio subfolders. `replaceInput` writes a hidden file
 * first and moves the original to the Trash only once that is done; it is
 * refused for animations made from several pictures.
 */
export type Destination =
  | { kind: "nextToInput" }
  | { kind: "folder"; path: string; byKind: boolean }
  | { kind: "replaceInput" };

/**
 * Making the picture bigger: `factor` multiplies both sides (1–8), `size` is
 * the short side to reach in pixels (2–16384, e.g. 1080 for 1080p). Never
 * shrinks; a target the picture already reaches changes nothing.
 */
export type Upscale = { factor: number } | { size: number };

/**
 * How pixels are resampled. `auto` is Lanczos for pictures, GIFs and any
 * enlargement, and ffmpeg's bicubic for shrinking video.
 */
export type Scaler = "auto" | "lanczos" | "spline" | "bicubic";

/** What became of an original a conversion replaced. */
export type Replaced = { to: "trash" } | { to: "renamed"; path: string };

export type ConvertOptions = {
  format: OutputFormat;
  trimStart: number | null;
  trimEnd: number | null;
  /** Short side in pixels; never upscales. Caps the result of `upscale` too. */
  maxSize: number | null;
  /** Enlarge the picture first; `null` keeps its size (shrink-only, as before). */
  upscale: Upscale | null;
  /** Resampling filter for any resize. */
  scaler: Scaler;
  fps: number | null;
  rotate: Rotation;
  flip: boolean;
  crop: Crop | null;
  /** 0 = smallest file, 100 = best quality. */
  quality: number;
  keepAudio: boolean;
  /** Codec of MP4, MOV, MKV and WebM files; `null` is the format's usual one. Ignored otherwise. */
  videoCodec: VideoCodec | null;
  /**
   * Exact average video bitrate in kbit/s instead of the CRF `quality` picks
   * (MP4, MOV, MKV, WebM); `null` uses `quality`. H.264, H.265 and VP9 take
   * two passes in software.
   */
  videoBitrate: number | null;
  /**
   * Encode H.265 on the graphics hardware where `FfmpegStatus.hevcHardware`
   * says it can (VideoToolbox on Apple silicon): much faster, bigger files.
   * Falls back to x265 elsewhere; ignored for other codecs and slideshows.
   */
  hardware: boolean;
  jxl: JxlOptions;
  sound: SoundOptions;
  /** Which dates, places and other tags to carry over, where the format can hold them. */
  metadata: MetadataMode;
  /** Playback speed of animations (GIF, animated WebP and JPEG XL), 0.25–4. Ignored otherwise. */
  speed: number;
  /** Playback direction of animations. Ignored otherwise. */
  direction: Direction;
  /** Where the result goes; next to the original when left out. */
  destination?: Destination;
};

/** How the sound is encoded: what the quality setting becomes for it. */
export type AudioRate =
  /** AAC or Opus aiming for this bitrate. */
  | { mode: "bitrate"; kbps: number }
  /** MP3 at LAME's VBR level (0 best to 7 smallest), averaging about `kbps`. */
  | { mode: "vbr"; level: number; kbps: number }
  /** FLAC. */
  | { mode: "lossless" }
  /** WAV: 16-bit PCM; `kbps` is `null` when the sample rate or channels are unknown. */
  | { mode: "pcm"; kbps: number | null };

/** How the picture of a video file is encoded. */
export type VideoRate = {
  codec: VideoCodec;
  /** Constant rate factor; `null` with an exact bitrate or in hardware. */
  crf: number | null;
  /** Exact average bitrate in kbit/s; `null` with a CRF. */
  kbps: number | null;
  /** 2 for an exact H.264, H.265 or VP9 bitrate in software, otherwise 1. */
  passes: number;
  /** Encoded by the graphics hardware (VideoToolbox) rather than x265. */
  hardware: boolean;
  /** VideoToolbox's constant quality (higher is better) in hardware without an exact bitrate. */
  hardwareQuality: number | null;
};

/** The numbers behind the quality setting, exactly as the planned command uses them. */
export type QualitySummary = {
  /** `null` when the output has no sound. */
  audio: AudioRate | null;
  /** `null` for outputs without a video codec: stills, GIFs, other animations, sound. */
  video: VideoRate | null;
};

/** How pictures of another shape fit an animation's frame (the first picture's size). */
export type Fit = "pad" | "crop";

/**
 * Settings that only matter when making an animation from pictures. The order
 * they play in is `ConvertOptions.direction`.
 */
export type SequenceOptions = {
  /** Seconds each picture shows (to the hundredth). */
  frameSecs: number;
  fit: Fit;
};

/** What to do when the chosen name is already taken. */
export type Collision =
  /** Keep the file that is there and number the new one: `clip 2.mp4`, … */
  | "keepBoth"
  /** Replace it once the new file is finished (never the original, nor a queued job's output). */
  | "overwrite";

/** Where a result is saved and under which name. All `null` saves next to the input as usual. */
export type OutputTarget = {
  /** Folder to save in; `null` is the input's folder. */
  folder: string | null;
  /**
   * File name without the extension, which follows the format. May hold
   * placeholders ({@link NAME_PLACEHOLDERS}), which makes it a pattern for batches.
   */
  name: string | null;
  collision: Collision;
};

/** Where a result would be saved, before it is. */
export type OutputPreview = {
  path: string;
  /** The chosen name is taken by a file that `"overwrite"` would replace; otherwise `path` has a number. */
  exists: boolean;
};

/** Saves next to the input under the usual free name. */
export const NEXT_TO_ORIGINAL: OutputTarget = { folder: null, name: null, collision: "keepBoth" };

/** Placeholders a name pattern can use (see `catconvert_core::PLACEHOLDERS`). */
export const NAME_PLACEHOLDERS = [
  { token: "{name}", meaning: "the original's name" },
  { token: "{format}", meaning: "the new extension, e.g. mp4" },
  { token: "{width}", meaning: "the new width in pixels" },
  { token: "{height}", meaning: "the new height in pixels" },
  { token: "{date}", meaning: "today's date" },
] as const;

export type JobId = number;

export type JobStage = "queued" | "running" | "done" | "failed" | "cancelled";

export type JobProgress = {
  jobId: JobId;
  stage: JobStage;
  /** Completion between 0 and 1. */
  progress: number;
  speed: number | null;
  outputBytes: number | null;
  /** Where the finished file is (`done` only); differs from the ticket's if that name was taken meanwhile. */
  output?: string | null;
  /** What became of the original it replaced (`done` only). */
  replaced?: Replaced | null;
  error: string | null;
};

/** A frame written by {@link saveFrame}. */
export type SavedFrame = {
  output: string;
  outputBytes: number;
};

/** Which half of a before/after comparison. */
export type SampleSide = "original" | "converted";

/** One half of a comparison, ready to show. */
export type Sample = {
  /** The picture, through the asset protocol. */
  url: string;
  /** Size of the encoded sample, or of the finished file; `null` for the original. */
  bytes: number | null;
  /** `bytes` is the size of the whole result, not just of a sample. */
  exact: boolean;
};

/** A sample as the backend reports it, before its path becomes a URL. */
type RawSample = { path: string; bytes: number | null; exact: boolean };

export type JobTicket = {
  id: JobId;
  output: string;
  /** The original the result will replace, if it does. */
  replaces?: string | null;
  /** The probed file, or an animation's pictures in order. */
  inputs: MediaInfo[];
  command: string;
};

export type FfmpegStatus = {
  found: boolean;
  /** The ffmpeg shipped inside the app rather than an installed one. */
  bundled: boolean;
  version: string | null;
  path: string | null;
  error: string | null;
  /** `animated`: pictures or a video can become an animation in this format. */
  formats: { format: OutputFormat; available: boolean; animated: boolean }[];
  /** Video codecs ffmpeg can write. */
  videoCodecs: VideoCodec[];
  /** H.265 can be encoded in hardware (VideoToolbox on Apple silicon). */
  hevcHardware: boolean;
  /** ffmpeg decodes AV1 video and AVIF pictures (it has dav1d or libaom). */
  readsAv1: boolean;
  /** Version of libjxl's cjxl/djxl, if installed. */
  libjxl: string | null;
  jxl: JxlSupport;
};

/** Whether "Convert with CatConvert" is in Finder's Quick Actions menu. */
export type QuickActionStatus = {
  /** Only macOS has Quick Actions. */
  supported: boolean;
  installed: boolean;
};

/** How a drag out of the window ended (tauri-plugin-drag). */
type DragEnd = { result: "Dropped" | "Cancel" };

export const NO_JXL: JxlSupport = {
  cjxl: false,
  djxl: false,
  ffmpegEncoder: false,
  ffmpegAnimEncoder: false,
  ffmpegDecoder: false,
  ffmpegAnimDecoder: false,
};

// ---------------------------------------------------------------------------
// Command and event contract
// ---------------------------------------------------------------------------

type Commands = {
  ffmpeg_status: { args: Record<string, never>; result: FfmpegStatus };
  relocate_ffmpeg: { args: Record<string, never>; result: FfmpegStatus };
  probe_file: { args: { path: string }; result: MediaInfo };
  preview_frame: {
    args: { path: string; at: number; height: number; hdr: boolean };
    result: ArrayBuffer;
  };
  compare_sample: {
    args: {
      info: MediaInfo;
      options: ConvertOptions;
      side: SampleSide;
      at: number;
      limit: number | null;
    };
    result: RawSample;
  };
  compare_file: {
    args: { path: string; at: number; limit: number | null; hdr: boolean };
    result: RawSample;
  };
  waveform: {
    args: { path: string; width: number; height: number; color: string };
    result: ArrayBuffer;
  };
  playback_source: { args: { path: string; decode: boolean }; result: string };
  loudness: { args: { path: string }; result: number | null };
  preview_command: {
    args: { info: MediaInfo; options: ConvertOptions; target: OutputTarget };
    result: string;
  };
  output_preview: {
    args: { info: MediaInfo; options: ConvertOptions; target: OutputTarget };
    result: OutputPreview;
  };
  sequence_output_preview: {
    args: { pictures: MediaInfo[]; options: ConvertOptions; target: OutputTarget };
    result: OutputPreview;
  };
  quality_summary: { args: { info: MediaInfo; options: ConvertOptions }; result: QualitySummary };
  check_command: {
    args: { info: MediaInfo; options: ConvertOptions; command: string; target: OutputTarget };
    result: string;
  };
  fill_command: {
    args: { info: MediaInfo; options: ConvertOptions; template: string; target: OutputTarget };
    result: string;
  };
  command_template: { args: { path: string; command: string }; result: string };
  start_custom_job: {
    args: { path: string; options: ConvertOptions; command: string; target: OutputTarget };
    result: JobTicket;
  };
  save_frame: {
    args: { path: string; options: ConvertOptions; at: number; target: OutputTarget };
    result: SavedFrame;
  };
  preview_sequence_command: {
    args: {
      pictures: MediaInfo[];
      options: ConvertOptions;
      sequence: SequenceOptions;
      target: OutputTarget;
    };
    result: string;
  };
  start_job: {
    args: { path: string; options: ConvertOptions; target: OutputTarget };
    result: JobTicket;
  };
  start_sequence: {
    args: {
      paths: string[];
      options: ConvertOptions;
      sequence: SequenceOptions;
      target: OutputTarget;
    };
    result: JobTicket;
  };
  cancel_job: { args: { id: JobId }; result: boolean };
  relaunch: { args: Record<string, never>; result: null };
  missing_files: { args: { paths: string[] }; result: string[] };
  default_output_folder: { args: Record<string, never>; result: string };
  prepare_output_folder: { args: { path: string }; result: string };
  third_party_licenses: { args: Record<string, never>; result: string | null };
  take_opened_files: { args: Record<string, never>; result: string[] };
  quick_action_status: { args: Record<string, never>; result: QuickActionStatus };
  set_quick_action: { args: { installed: boolean }; result: QuickActionStatus };
  set_window_theme: { args: { theme: ThemeChoice }; result: null };
  "plugin:drag|start_drag": {
    args: { item: string[]; image: string; options: { mode: "copy" }; onEvent: Channel<DragEnd> };
    result: null;
  };
};

type Events = {
  "job-progress": JobProgress;
  /** Absolute paths of files opened from outside the app. */
  "open-files": string[];
};

function command<K extends keyof Commands>(
  name: K,
  args: Commands[K]["args"],
): Promise<Commands[K]["result"]> {
  return invoke<Commands[K]["result"]>(name, args);
}

function subscribe<K extends keyof Events>(
  name: K,
  handler: (payload: Events[K]) => void,
): Promise<UnlistenFn> {
  return listen<Events[K]>(name, (event) => handler(event.payload));
}

/** Whether we run inside the Tauri shell (false in a plain browser or jsdom). */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function blobUrl(bytes: ArrayBuffer, type: string): string {
  return URL.createObjectURL(new Blob([bytes], { type }));
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export async function ffmpegStatus(): Promise<FfmpegStatus> {
  if (!inTauri) {
    return {
      found: false,
      bundled: false,
      version: null,
      path: null,
      error: "Open CatConvert as an app to convert files.",
      formats: [],
      videoCodecs: [],
      hevcHardware: false,
      readsAv1: false,
      libjxl: null,
      jxl: NO_JXL,
    };
  }
  return command("ffmpeg_status", {});
}

/** Search for ffmpeg again, e.g. after the user installed it. */
export function relocateFfmpeg(): Promise<FfmpegStatus> {
  return command("relocate_ffmpeg", {});
}

/** Probe a file. Rejects with the backend's error message (a string). */
export function probeFile(path: string): Promise<MediaInfo> {
  return command("probe_file", { path });
}

/**
 * One JPEG frame as an object URL, tone-mapped when `hdr` is set. Revoke it
 * with `URL.revokeObjectURL` when done.
 */
export async function previewFrame(
  path: string,
  at: number,
  height: number,
  hdr = false,
): Promise<string> {
  return blobUrl(await command("preview_frame", { path, at, height, hdr }), "image/jpeg");
}

function sampleFrom(raw: RawSample): Sample {
  return { url: convertFileSrc(raw.path), bytes: raw.bytes, exact: raw.exact };
}

/**
 * One half of a before/after comparison of converting `info` with `options`:
 * the original (turned, mirrored and cropped, but not encoded) or what the
 * settings make of it, at the frame `at` seconds in. `limit` caps the short
 * side in pixels; `null` renders at full size.
 */
export async function compareSample(
  info: MediaInfo,
  options: ConvertOptions,
  side: SampleSide,
  at: number,
  limit: number | null,
): Promise<Sample> {
  return sampleFrom(await command("compare_sample", { info, options, side, at, limit }));
}

/**
 * The frame of a finished file at `at` seconds, to compare it with its
 * original. `hdr` tone-maps an HDR video the way its original is shown.
 */
export async function compareFile(
  path: string,
  at: number,
  limit: number | null,
  hdr: boolean,
): Promise<Sample> {
  return sampleFrom(await command("compare_file", { path, at, limit, hdr }));
}

/**
 * Save the frame at `at` seconds as a PNG where `target` says (by default next
 * to the video), with the turn, mirror and size of `options`. Runs right away,
 * not in the queue.
 */
export function saveFrame(
  path: string,
  options: ConvertOptions,
  at: number,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<SavedFrame> {
  return command("save_frame", { path, options, at, target });
}

/** The audio waveform as a PNG object URL. */
export async function waveform(
  path: string,
  width: number,
  height: number,
  color: string,
): Promise<string> {
  return blobUrl(await command("waveform", { path, width, height, color }), "image/png");
}

/**
 * A URL an `<audio>` element can play `path` from (Tauri's asset protocol).
 * With `decode`, the sound is first decoded to a temporary WAV file, for
 * formats the webview can't play itself.
 */
export async function playbackUrl(path: string, decode = false): Promise<string> {
  if (!inTauri) throw new Error("Open CatConvert as an app to play files.");
  return convertFileSrc(await command("playback_source", { path, decode }));
}

/** Integrated loudness of the sound in LUFS, or `null` when it is silent. */
export function loudness(path: string): Promise<number | null> {
  return command("loudness", { path });
}

/** The command(s) a conversion would run. */
export function previewCommand(
  info: MediaInfo,
  options: ConvertOptions,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<string> {
  return command("preview_command", { info, options, target });
}

/**
 * Where converting `info` with `options` would save to right now. Rejects
 * with what is wrong with the chosen name or folder.
 */
export function outputPreview(
  info: MediaInfo,
  options: ConvertOptions,
  target: OutputTarget,
): Promise<OutputPreview> {
  return command("output_preview", { info, options, target });
}

/** Like {@link outputPreview}, for an animation made from `pictures`. */
export function sequenceOutputPreview(
  pictures: MediaInfo[],
  options: ConvertOptions,
  target: OutputTarget,
): Promise<OutputPreview> {
  return command("sequence_output_preview", { pictures, options, target });
}

/** The audio bitrate and video CRF (or exact bitrate) a conversion would use. */
export function qualitySummary(info: MediaInfo, options: ConvertOptions): Promise<QualitySummary> {
  return command("quality_summary", { info, options });
}

/** Queue a conversion; progress arrives via {@link onJobProgress}. */
export function startJob(
  path: string,
  options: ConvertOptions,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<JobTicket> {
  return command("start_job", { path, options, target });
}

/**
 * Check a hand-edited command without running it. Resolves to the command as
 * it would run; rejects with why it would be refused (another program, a
 * shell feature, writing anywhere but the output, …).
 */
export function checkCommand(
  info: MediaInfo,
  options: ConvertOptions,
  text: string,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<string> {
  return command("check_command", { info, options, command: text, target });
}

/** A recipe's saved command (see {@link commandTemplate}) with this file's paths filled in. */
export function fillCommand(
  info: MediaInfo,
  options: ConvertOptions,
  template: string,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<string> {
  return command("fill_command", { info, options, template, target });
}

/** An edited command with the paths of the file at `path` swapped for placeholders. */
export function commandTemplate(path: string, text: string): Promise<string> {
  return command("command_template", { path, command: text });
}

/**
 * Queue a conversion running `text`, a command edited by hand (or a template from
 * {@link commandTemplate}), instead of the generated one. `options` still pick
 * the output name. Rejects, without queueing, if the command is refused.
 */
export function startCustomJob(
  path: string,
  options: ConvertOptions,
  text: string,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<JobTicket> {
  return command("start_custom_job", { path, options, command: text, target });
}

/** The command(s) making one animation from `pictures` would run. */
export function previewSequenceCommand(
  pictures: MediaInfo[],
  options: ConvertOptions,
  sequence: SequenceOptions,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<string> {
  return command("preview_sequence_command", { pictures, options, sequence, target });
}

/** Queue one animation made from the pictures at `paths`, in this order. */
export function startSequence(
  paths: string[],
  options: ConvertOptions,
  sequence: SequenceOptions,
  target: OutputTarget = NEXT_TO_ORIGINAL,
): Promise<JobTicket> {
  return command("start_sequence", { paths, options, sequence, target });
}

export function cancelJob(id: JobId): Promise<boolean> {
  return command("cancel_job", { id });
}

/** License texts of the bundled ffmpeg and libjxl tools; `null` in builds without them. */
export async function thirdPartyLicenses(): Promise<string | null> {
  if (!inTauri) return null;
  return command("third_party_licenses", {});
}

export function onJobProgress(handler: (event: JobProgress) => void): Promise<UnlistenFn> {
  return subscribe("job-progress", handler);
}

/** The paths that no longer point at a file (moved or deleted). */
export async function missingFiles(paths: string[]): Promise<string[]> {
  if (!inTauri || paths.length === 0) return [];
  return command("missing_files", { paths });
}

/** Native multi-file picker. Resolves to the chosen paths (empty if cancelled). */
export async function pickFiles(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    directory: false,
    filters: [
      {
        name: "Media",
        extensions: [
          ...["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpg", "mpeg", "gif", "apng"],
          ...["mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "aiff", "wma"],
          ...["jpg", "jpeg", "png", "webp", "avif", "jxl", "tif", "tiff", "bmp"],
          ...["heic", "heif", "hif"],
        ],
      },
    ],
  });
  if (picked === null) return [];
  return Array.isArray(picked) ? picked : [picked];
}

/**
 * Native save dialog for a file of `format`, starting at `suggested` (a full
 * path). Resolves to the chosen path, or `null` if cancelled. The system asks
 * before a chosen file would be replaced.
 */
export async function chooseSavePath(
  suggested: string,
  format: OutputFormat,
  label: string,
): Promise<string | null> {
  return save({
    title: "Save as",
    defaultPath: suggested,
    filters: [{ name: label, extensions: [format] }],
  });
}

/** The answer to {@link askReplace}. */
export type ReplaceChoice = "replace" | "keepBoth" | "cancel";

/** Ask whether to replace `name`, which already exists in `folder`, keep both, or not save. */
export async function askReplace(name: string, folder: string): Promise<ReplaceChoice> {
  const buttons = { yes: "Replace", no: "Keep both", cancel: "Cancel" };
  const answer = await message(
    `“${name}” already exists in ${folder}. Replace it, or keep both and number the new file?`,
    { title: "File already exists", kind: "warning", buttons },
  );
  if (answer === buttons.yes || answer === "Yes") return "replace";
  if (answer === buttons.no || answer === "No") return "keepBoth";
  return "cancel";
}

/** Native folder picker. Resolves to the chosen folder, or `null` if cancelled. */
export async function pickFolder(defaultPath: string | null = null): Promise<string | null> {
  const picked = await open({
    multiple: false,
    directory: true,
    ...(defaultPath ? { defaultPath } : {}),
  });
  return Array.isArray(picked) ? (picked[0] ?? null) : picked;
}

/** The folder Settings suggests for converted files (`CatConvert` in Downloads). */
export async function defaultOutputFolder(): Promise<string | null> {
  if (!inTauri) return null;
  return command("default_output_folder", {});
}

/** Create the output folder if need be and check files can be saved there. */
export function prepareOutputFolder(path: string): Promise<string> {
  return command("prepare_output_folder", { path });
}

/** Show a file in Finder / Explorer / the file manager. */
export function revealInFolder(path: string): Promise<void> {
  return revealItemInDir(path);
}

// ---------------------------------------------------------------------------
// Files from and to other apps
// ---------------------------------------------------------------------------

/**
 * Files opened from outside the app: "Open with", the Finder Quick Action,
 * "Send to", a drop on the Dock icon or the command line. `handler` first gets
 * the files that arrived before the window was ready, then each new batch.
 */
export async function onOpenedFiles(handler: (paths: string[]) => void): Promise<UnlistenFn> {
  // Subscribe first: the backend emits events once the backlog is taken.
  const stop = await subscribe("open-files", handler);
  try {
    const backlog = await command("take_opened_files", {});
    if (backlog.length > 0) handler(backlog);
  } catch (error) {
    stop();
    throw error;
  }
  return stop;
}

export async function quickActionStatus(): Promise<QuickActionStatus> {
  if (!inTauri) return { supported: false, installed: false };
  return command("quick_action_status", {});
}

/** Add or remove the Finder Quick Action. */
export function setQuickAction(installed: boolean): Promise<QuickActionStatus> {
  return command("set_quick_action", { installed });
}

/** The app's look: follow the system, or always light or dark. */
export type ThemeChoice = "system" | "light" | "dark";

/**
 * Give the native window (title bar, background behind the page) the theme,
 * remembered for the next launch so the window opens in it, and show the
 * window if it is still hidden: it starts hidden so it never flashes white.
 */
export async function setWindowTheme(theme: ThemeChoice): Promise<void> {
  if (!inTauri) return;
  await command("set_window_theme", { theme });
}

/** A drag out of the window is in progress, so a drop on it is our own file coming back. */
let draggingOut = false;

/**
 * Drag files out of the window into another app or folder, as copies. HTML
 * drag and drop can't hand real files to other apps, so this starts a native
 * drag; call it while the mouse button is down. `icon` is a PNG data URL shown
 * under the pointer. Resolves when the drag ends.
 */
export function dragFiles(paths: string[], icon: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const end = () => {
      draggingOut = false;
      window.removeEventListener("pointerdown", end);
      resolve();
    };
    const onEvent = new Channel<DragEnd>();
    onEvent.onmessage = end;
    // The window gets no pointer events during a native drag, so the next
    // press means it is over, should the plugin never report back.
    window.addEventListener("pointerdown", end);
    draggingOut = true;
    command("plugin:drag|start_drag", {
      item: paths,
      image: icon,
      options: { mode: "copy" },
      onEvent,
    }).catch((error: unknown) => {
      draggingOut = false;
      window.removeEventListener("pointerdown", end);
      reject(error);
    });
  });
}

// ---------------------------------------------------------------------------
// Updates
// ---------------------------------------------------------------------------

/** A newer release found by {@link checkForUpdate}. */
export type UpdateInfo = {
  version: string;
  currentVersion: string;
  /** Release date as reported by `latest.json`, if any. */
  date: string | null;
  /** Release notes, if any. */
  notes: string | null;
};

/** Download progress; `total` is null when the server sends no length. */
export type DownloadProgress = { downloaded: number; total: number | null };

/** The update last returned by {@link checkForUpdate}, kept for {@link installUpdate}. */
let pendingUpdate: Update | null = null;

/** The running app's version, or null outside the Tauri shell. */
export async function appVersion(): Promise<string | null> {
  return inTauri ? getVersion() : null;
}

/**
 * Ask the release server whether a newer version exists. Resolves to null when
 * the app is up to date (or not running in the Tauri shell).
 */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  if (!inTauri) return null;
  const update = await check();
  const previous = pendingUpdate;
  pendingUpdate = update;
  if (previous) void previous.close().catch(() => {});
  if (!update) return null;
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    date: update.date ?? null,
    notes: update.body?.trim() || null,
  };
}

/**
 * Download, verify and install the update found by the last
 * {@link checkForUpdate}. On Windows the installer closes the app itself;
 * elsewhere call {@link relaunch} afterwards.
 */
export async function installUpdate(onProgress: (progress: DownloadProgress) => void) {
  const update = pendingUpdate;
  if (!update) throw new Error("There is no update to install. Check again.");
  let downloaded = 0;
  let total: number | null = null;
  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? null;
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        break;
      case "Finished":
        if (total !== null) downloaded = total;
        break;
    }
    onProgress({ downloaded, total });
  });
}

/** Restart the app (to finish installing an update). */
export async function relaunch(): Promise<void> {
  await command("relaunch", {});
}

export type FileDropHandlers = {
  /** Called with `true` while files hover over the window, `false` when they leave or drop. */
  onHover?: (hovering: boolean) => void;
  /** Called with the absolute paths of dropped files. */
  onDrop: (paths: string[]) => void;
};

/**
 * Subscribe to native drag-and-drop on the current webview (gives real file
 * paths). Files dragged out with {@link dragFiles} and let go over the window
 * again are ignored.
 */
export function onFileDrop(handlers: FileDropHandlers): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    switch (payload.type) {
      case "enter":
      case "over":
        if (!draggingOut) handlers.onHover?.(true);
        break;
      case "leave":
        handlers.onHover?.(false);
        break;
      case "drop":
        handlers.onHover?.(false);
        if (!draggingOut) handlers.onDrop(payload.paths);
        break;
    }
  });
}
