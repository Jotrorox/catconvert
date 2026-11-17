/**
 * Helpers for where a result is saved and what it is called. The backend
 * decides the final path (`outputPreview`); these only take paths apart and
 * explain names as they are typed.
 */
import type { Collision, OutputFormat, OutputTarget } from "./api";

/** The editor's choice of output: like an {@link OutputTarget}, but `collision` is `null` until asked. */
export type OutputChoice = {
  folder: string | null;
  name: string | null;
  /** `null`: ask before replacing a file, if the chosen name turns out to be taken. */
  collision: Collision | null;
};

/** Next to the original, as usual; asks nothing. */
export const USUAL_OUTPUT: OutputChoice = { folder: null, name: null, collision: null };

/** Extensions the core reads as media (`MediaKind::from_extension`). */
const MEDIA_EXTENSIONS = new Set([
  ...["jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "heif", "hif", "tif", "tiff"],
  ...["bmp", "jxl", "apng"],
  ...["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpg", "mpeg"],
  ...["mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "aiff", "wma"],
]);

/** Whether the choice saves next to the original under the usual name. */
export function isUsual(choice: { folder: string | null; name: string | null }): boolean {
  return choice.folder === null && (choice.name ?? "").trim() === "";
}

/** The choice as the backend takes it; an unasked collision keeps both files. */
export function targetOf(choice: OutputChoice): OutputTarget {
  const name = choice.name?.trim() ? choice.name : null;
  return { folder: choice.folder, name, collision: choice.collision ?? "keepBoth" };
}

/** Whether a name has placeholders like `{name}`, which makes it a pattern. */
export function isPattern(name: string | null): boolean {
  return /\{[A-Za-z0-9]+\}/.test(name ?? "");
}

/** The folder part of a path (`/a/b.mp4` → `/a`), with either separator. */
export function dirName(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (cut < 0) return "";
  // Keep the root: `/b.mp4` → `/`, `C:\b.mp4` → `C:\`.
  return cut === 0 || path[cut - 1] === ":" ? path.slice(0, cut + 1) : path.slice(0, cut);
}

/** The trailing media extension of `name`, lower-cased, if it has one: `clip.MOV` → `mov`. */
export function mediaExtension(name: string): string | null {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || name.slice(0, dot).trim() === "") return null;
  const ext = name.slice(dot + 1).toLowerCase();
  return MEDIA_EXTENSIONS.has(ext) ? ext : null;
}

/** `name` without a trailing media extension, as the backend saves it: the format decides. */
export function withoutExtension(name: string): string {
  const ext = mediaExtension(name);
  return ext ? name.slice(0, name.length - ext.length - 1).trimEnd() : name;
}

/**
 * Why a typed name won't be saved quite as typed, if it won't: it ends in
 * another format's extension.
 */
export function extensionNote(name: string, format: OutputFormat): string | null {
  const ext = mediaExtension(name.trim());
  if (!ext || ext === format || (ext === "jpeg" && format === "jpg")) return null;
  return `The format decides the extension, so it's saved as .${format}, not .${ext}.`;
}

/** Split a chosen file path into its folder and its name without a media extension. */
export function splitChosen(path: string): { folder: string; name: string } {
  const folder = dirName(path);
  const file = path.slice(folder.length).replace(/^[\\/]/, "");
  return { folder, name: withoutExtension(file) };
}
