/**
 * Typed bridge to the Tauri backend.
 *
 * Every `invoke` and `listen` call in the app goes through this module, so the
 * command/event contract with `apps/desktop/src-tauri/src/lib.rs` lives in one
 * place. The types mirror the serde output of `catconvert-core` (camelCase).
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

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
};

export type JobId = number;

export type JobStage = "queued" | "running" | "done";

export type JobProgress = {
  jobId: JobId;
  stage: JobStage;
  /** Completion between 0 and 1. */
  progress: number;
};

// ---------------------------------------------------------------------------
// Command and event contract
// ---------------------------------------------------------------------------

type Commands = {
  probe_file: { args: { path: string }; result: MediaInfo };
  start_job: { args: { path: string; targetFormat: string }; result: JobId };
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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/** Probe a file. Rejects with the backend's error message (a string). */
export function probeFile(path: string): Promise<MediaInfo> {
  return command("probe_file", { path });
}

/** Start a (stubbed) conversion job; progress arrives via {@link onJobProgress}. */
export function startJob(path: string, targetFormat: string): Promise<JobId> {
  return command("start_job", { path, targetFormat });
}

export function onJobProgress(handler: (event: JobProgress) => void): Promise<UnlistenFn> {
  return subscribe("job-progress", handler);
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
