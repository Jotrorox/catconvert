import type { JobId, JobStage, MediaInfo, MediaKind } from "./api";

/** One dropped file as tracked by the UI. */
export type FileEntry = {
  path: string;
  status: "probing" | "ready" | "error";
  info?: MediaInfo;
  error?: string;
  job?: { id: JobId; stage: JobStage; progress: number };
};

/** Placeholder target formats until real conversion settings exist. */
export const DEFAULT_TARGET: Record<MediaKind, string | null> = {
  image: "webp",
  video: "mp4",
  audio: "mp3",
  unknown: null,
};

/** Last path segment, handling both `/` and `\` separators. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}
