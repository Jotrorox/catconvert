/**
 * Typed bridge to the Tauri backend.
 *
 * Every `invoke` and `listen` call in the app goes through this module, so the
 * command/event contract with `apps/desktop/src-tauri/src/lib.rs` lives in one
 * place. The types mirror the serde output of `catconvert-core` (camelCase).
 */
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
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
  /** JPEG XL only: a losslessly recompressed JPEG whose original can be restored. */
  reconstructsJpeg: boolean;
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

export type Rotation = 0 | 90 | 180 | 270;

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

export type ConvertOptions = {
  format: OutputFormat;
  trimStart: number | null;
  trimEnd: number | null;
  /** Short side in pixels; never upscales. */
  maxSize: number | null;
  fps: number | null;
  rotate: Rotation;
  flip: boolean;
  /** 0 = smallest file, 100 = best quality. */
  quality: number;
  keepAudio: boolean;
  jxl: JxlOptions;
  sound: SoundOptions;
  /** Carry over dates, location and other tags where the format can hold them. */
  keepMetadata: boolean;
};

export type JobId = number;

export type JobStage = "queued" | "running" | "done" | "failed" | "cancelled";

export type JobProgress = {
  jobId: JobId;
  stage: JobStage;
  /** Completion between 0 and 1. */
  progress: number;
  speed: number | null;
  outputBytes: number | null;
  error: string | null;
};

export type JobTicket = {
  id: JobId;
  output: string;
  input: MediaInfo;
  command: string;
};

export type FfmpegStatus = {
  found: boolean;
  /** The ffmpeg shipped inside the app rather than an installed one. */
  bundled: boolean;
  version: string | null;
  path: string | null;
  error: string | null;
  formats: { format: OutputFormat; available: boolean }[];
  /** Version of libjxl's cjxl/djxl, if installed. */
  libjxl: string | null;
  jxl: JxlSupport;
};

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
  preview_frame: { args: { path: string; at: number; height: number }; result: ArrayBuffer };
  waveform: {
    args: { path: string; width: number; height: number; color: string };
    result: ArrayBuffer;
  };
  preview_command: { args: { info: MediaInfo; options: ConvertOptions }; result: string };
  start_job: { args: { path: string; options: ConvertOptions }; result: JobTicket };
  cancel_job: { args: { id: JobId }; result: boolean };
  relaunch: { args: Record<string, never>; result: null };
  missing_files: { args: { paths: string[] }; result: string[] };
  third_party_licenses: { args: Record<string, never>; result: string | null };
};

type Events = {
  "job-progress": JobProgress;
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

/** One JPEG frame as an object URL. Revoke it with `URL.revokeObjectURL` when done. */
export async function previewFrame(path: string, at: number, height: number): Promise<string> {
  return blobUrl(await command("preview_frame", { path, at, height }), "image/jpeg");
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

/** The command(s) a conversion would run. */
export function previewCommand(info: MediaInfo, options: ConvertOptions): Promise<string> {
  return command("preview_command", { info, options });
}

/** Queue a conversion; progress arrives via {@link onJobProgress}. */
export function startJob(path: string, options: ConvertOptions): Promise<JobTicket> {
  return command("start_job", { path, options });
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
          ...["jpg", "jpeg", "png", "webp", "avif", "jxl", "heic", "heif", "tif", "tiff", "bmp"],
        ],
      },
    ],
  });
  if (picked === null) return [];
  return Array.isArray(picked) ? picked : [picked];
}

/** Show a file in Finder / Explorer / the file manager. */
export function revealInFolder(path: string): Promise<void> {
  return revealItemInDir(path);
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

/** Subscribe to native drag-and-drop on the current webview (gives real file paths). */
export function onFileDrop(handlers: FileDropHandlers): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    switch (payload.type) {
      case "enter":
      case "over":
        handlers.onHover?.(true);
        break;
      case "leave":
        handlers.onHover?.(false);
        break;
      case "drop":
        handlers.onHover?.(false);
        handlers.onDrop(payload.paths);
        break;
    }
  });
}
