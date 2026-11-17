/**
 * Typed bridge to the Tauri backend.
 *
 * Every `invoke` and `listen` call in the app goes through this module, so the
 * command/event contract with `apps/desktop/src-tauri/src/lib.rs` lives in one
 * place. The types mirror the serde output of `catconvert-core` (camelCase).
 */
import { getVersion } from "@tauri-apps/api/app";
import { Channel, invoke } from "@tauri-apps/api/core";
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
  /** HDR video (PQ or HLG, as iPhones record); pictures made from it are tone-mapped. */
  hdr: boolean;
  /** GPS in a photo's EXIF or XMP, or a location tag in a video or song. */
  hasLocation: boolean;
  /** Container tags holding the location, cleared when everything else is kept. */
  locationTags: string[];
  /** HEIF only: the stream group ffmpeg stitches a tiled photo from. */
  tileGrid: number | null;
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

export type ConvertOptions = {
  format: OutputFormat;
  trimStart: number | null;
  trimEnd: number | null;
  /** Short side in pixels; never upscales. */
  maxSize: number | null;
  fps: number | null;
  rotate: Rotation;
  flip: boolean;
  crop: Crop | null;
  /** 0 = smallest file, 100 = best quality. */
  quality: number;
  keepAudio: boolean;
  jxl: JxlOptions;
  sound: SoundOptions;
  /** Which dates, places and other tags to carry over, where the format can hold them. */
  metadata: MetadataMode;
  /** Playback speed of animations (GIF, animated JPEG XL), 0.25–4. Ignored otherwise. */
  speed: number;
  /** Playback direction of animations. Ignored otherwise. */
  direction: Direction;
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

/** A frame written by {@link saveFrame}. */
export type SavedFrame = {
  output: string;
  outputBytes: number;
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
  waveform: {
    args: { path: string; width: number; height: number; color: string };
    result: ArrayBuffer;
  };
  preview_command: { args: { info: MediaInfo; options: ConvertOptions }; result: string };
  save_frame: { args: { path: string; options: ConvertOptions; at: number }; result: SavedFrame };
  start_job: { args: { path: string; options: ConvertOptions }; result: JobTicket };
  cancel_job: { args: { id: JobId }; result: boolean };
  relaunch: { args: Record<string, never>; result: null };
  missing_files: { args: { paths: string[] }; result: string[] };
  third_party_licenses: { args: Record<string, never>; result: string | null };
  take_opened_files: { args: Record<string, never>; result: string[] };
  quick_action_status: { args: Record<string, never>; result: QuickActionStatus };
  set_quick_action: { args: { installed: boolean }; result: QuickActionStatus };
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

/**
 * Save the frame at `at` seconds as a PNG next to the video, with the turn,
 * mirror and size of `options`. Runs right away, not in the queue.
 */
export function saveFrame(path: string, options: ConvertOptions, at: number): Promise<SavedFrame> {
  return command("save_frame", { path, options, at });
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
          ...["jpg", "jpeg", "png", "webp", "avif", "jxl", "tif", "tiff", "bmp"],
          ...["heic", "heif", "hif"],
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
