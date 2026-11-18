/**
 * Before/after comparison: what to call each half, which samples to ask the
 * backend for, and the zoom and pan arithmetic the compare view shares
 * between its halves.
 *
 * The backend renders both halves (`compare_sample` in the Tauri shell,
 * `Samples` in crates/core/src/sample.rs): the original with the turn,
 * mirror and crop applied but not encoded, and the converted half encoded
 * with the current settings, usually at {@link PREVIEW_LIMIT} so it keeps up
 * with the quality slider.
 */
import { type Accessor, createEffect, createSignal, on, onCleanup } from "solid-js";
import type { ConvertOptions, MediaInfo, OutputFormat, Sample } from "./api";
import type { Size } from "./crop";
import { estimateBytes } from "./estimate";
import { formatBitrate, formatBytes } from "./format";
import {
  CODEC_LABELS,
  codecOf,
  formatKind,
  isAnimatedImage,
  isLossless,
  label,
  repacksJpeg,
  restoresJpeg,
  videoCodecs,
} from "./formats";

/** Short side of samples while the picture is fitted; zooming in asks for full size. */
export const PREVIEW_LIMIT = 1080;

/** Wipe: one picture with a divider. Side: two pictures next to each other. */
export type CompareLayout = "wipe" | "side";

/** How the compare view lays out its halves right now; phones flip between them. */
export type CompareMode = CompareLayout | "flip";

/** Fit the picture in, or show it at 100% or 200% (picture pixels per CSS pixel). */
export type Zoom = "fit" | 1 | 2;

export const ZOOMS: readonly { zoom: Zoom; label: string }[] = [
  { zoom: "fit", label: "Fit" },
  { zoom: 1, label: "100%" },
  { zoom: 2, label: "200%" },
];

/** Whether converting `info` to `format` makes something to look at. */
export function canCompare(info: Pick<MediaInfo, "kind">, format: OutputFormat): boolean {
  return (
    info.kind !== "audio" && info.kind !== "unknown" && formatKind(format, info.kind) !== "audio"
  );
}

/** "WebP q73", "MP4 H.265 q60", "MP4 4 Mbps", "JXL lossless", "PNG". */
export function convertedName(info: MediaInfo, options: ConvertOptions): string {
  const parts = [label(options.format)];
  const codec = isAnimatedImage(options.format, info.kind) ? null : codecOf(options);
  if (codec && codec !== videoCodecs(options.format)[0]) parts.push(CODEC_LABELS[codec]);
  if (options.format === "png") return parts.join(" ");
  if (isLossless(options) || restoresJpeg(info, options) || repacksJpeg(info, options)) {
    parts.push("lossless");
  } else if (codec && options.videoBitrate) {
    parts.push(formatBitrate(options.videoBitrate * 1000));
  } else {
    parts.push(`q${options.quality}`);
  }
  return parts.join(" ");
}

/**
 * The converted half's size: exact when the sample is the whole result,
 * otherwise the editor's estimate ("~610 KB"). `null` when unknown.
 */
export function convertedSize(
  info: MediaInfo,
  options: ConvertOptions,
  sample: Pick<Sample, "bytes" | "exact"> | null,
): string | null {
  if (sample?.exact && sample.bytes !== null) return formatBytes(sample.bytes);
  const guess = estimateBytes(info, options);
  if (guess === null || !Number.isFinite(guess)) return null;
  return `~${formatBytes(roughly(guess)).replace(".0 ", " ")}`;
}

/** Two significant digits, so a guess doesn't promise precision. */
function roughly(bytes: number): number {
  if (bytes <= 0) return 0;
  const scale = 10 ** (Math.floor(Math.log10(bytes)) - 1);
  return Math.round(bytes / scale) * scale;
}

/** Label of the original half: "Original · 4.2 MB". */
export function originalLabel(info: Pick<MediaInfo, "sizeBytes">): string {
  return `Original · ${formatBytes(info.sizeBytes)}`;
}

/** What a sample request depends on, so equal requests are asked once. */
export type SampleRequest = { key: string; run: () => Promise<Sample> };

/**
 * Key for the original half. Only the picture's geometry matters, and for
 * videos the format too (a video and a picture made from it are sampled
 * differently).
 */
export function originalKey(
  info: MediaInfo,
  options: ConvertOptions,
  at: number,
  limit: number | null,
): string {
  const video = info.kind === "video";
  return JSON.stringify([
    info.path,
    video ? at : 0,
    limit,
    options.rotate,
    options.flip,
    options.crop,
    video ? options.fps : null,
    video ? options.format : null,
  ]);
}

/**
 * Key for the converted half: everything that changes how it is encoded.
 * Where it is saved, the trim, sound and playback changes don't.
 */
export function convertedKey(
  info: MediaInfo,
  options: ConvertOptions,
  at: number,
  limit: number | null,
): string {
  const {
    destination: _destination,
    trimStart: _start,
    trimEnd: _end,
    keepAudio: _audio,
    sound: _sound,
    speed: _speed,
    direction: _direction,
    ...look
  } = options;
  return JSON.stringify([info.path, info.kind === "video" ? at : 0, limit, look]);
}

/** Samples kept per half, so going back to a setting or zoom shows at once. */
const CACHED_SAMPLES = 4;

export type SampleState = {
  sample: Sample | null;
  /** A newer sample is on its way; `sample` is the previous one. */
  busy: boolean;
  error: string | null;
};

/**
 * The sample for `request()`, re-asked `delay` ms after the request last
 * changed. One render runs at a time and only the newest request waits, so
 * dragging a slider never piles up ffmpeg runs; the last sample stays up
 * until the next arrives. `retry` asks again once for a sample whose picture
 * didn't load (e.g. it was cleaned up).
 */
export function createSample(
  request: Accessor<SampleRequest | null>,
  delay = 150,
): { state: Accessor<SampleState>; retry: () => void } {
  const [state, setState] = createSignal<SampleState>({ sample: null, busy: false, error: null });
  const cache = new Map<string, Sample>();
  const retried = new Set<string>();
  let wanted: SampleRequest | null = null;
  let running = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
    clearTimeout(timer);
  });

  const remember = (key: string, sample: Sample) => {
    cache.delete(key);
    cache.set(key, sample);
    while (cache.size > CACHED_SAMPLES) {
      const oldest = cache.keys().next().value;
      if (oldest === undefined) break;
      cache.delete(oldest);
    }
  };

  const pump = async () => {
    if (running) return;
    running = true;
    while (wanted && alive) {
      const job = wanted;
      try {
        const sample = await job.run();
        if (!alive) break;
        remember(job.key, sample);
        if (wanted === job) {
          wanted = null;
          setState({ sample, busy: false, error: null });
        }
      } catch (error) {
        if (!alive) break;
        if (wanted === job) {
          wanted = null;
          setState((s) => ({ ...s, busy: false, error: String(error) }));
        }
      }
    }
    running = false;
  };

  const ask = (next: SampleRequest | null) => {
    clearTimeout(timer);
    if (!next) {
      wanted = null;
      setState((s) => ({ ...s, busy: false }));
      return;
    }
    const hit = cache.get(next.key);
    if (hit) {
      wanted = null;
      remember(next.key, hit);
      setState({ sample: hit, busy: false, error: null });
      return;
    }
    wanted = next;
    setState((s) => ({ ...s, busy: true }));
    timer = setTimeout(() => void pump(), delay);
  };

  createEffect(
    on(
      () => request()?.key ?? null,
      () => ask(request()),
    ),
  );

  const retry = () => {
    const current = request();
    if (!current || retried.has(current.key)) return;
    retried.add(current.key);
    cache.delete(current.key);
    ask(current);
  };
  return { state, retry };
}

// ---------------------------------------------------------------------------
// Zoom and pan, shared by both halves
// ---------------------------------------------------------------------------

export type Point = { x: number; y: number };

/** CSS pixels per picture pixel at `zoom` for a picture in a `stage`. */
export function scaleFor(zoom: Zoom, stage: Size, picture: Size): number {
  if (zoom !== "fit") return zoom;
  if (picture.width <= 0 || picture.height <= 0) return 1;
  return Math.min(stage.width / picture.width, stage.height / picture.height);
}

/**
 * Keep a picture of `shown` size covering the stage once it is bigger:
 * `pan` is how far its centre sits from the stage's centre.
 */
export function clampPan(pan: Point, stage: Size, shown: Size): Point {
  const room = (picture: number, box: number) => Math.max(0, (picture - box) / 2);
  const clamp = (v: number, max: number) => Math.min(max, Math.max(-max, v));
  return {
    x: clamp(pan.x, room(shown.width, stage.width)),
    y: clamp(pan.y, room(shown.height, stage.height)),
  };
}

/**
 * The pan that keeps the picture point under `at` (stage coordinates from
 * its centre) in place when the scale goes from `from` to `to`.
 */
export function panAround(pan: Point, at: Point, from: number, to: number): Point {
  const ratio = from > 0 ? to / from : 1;
  return { x: at.x - (at.x - pan.x) * ratio, y: at.y - (at.y - pan.y) * ratio };
}

// ---------------------------------------------------------------------------
// Finished files
// ---------------------------------------------------------------------------

/**
 * The time in the original that a finished file shows `t` seconds in. A
 * faster animation covers more of the original per second, a reversed one
 * plays it from the end, a boomerang forward then back.
 */
export function sourceTime(
  options: Pick<ConvertOptions, "trimStart" | "trimEnd" | "speed" | "direction">,
  duration: number | null,
  t: number,
): number {
  const start = options.trimStart ?? 0;
  const end = options.trimEnd ?? duration ?? Number.POSITIVE_INFINITY;
  const speed = options.speed > 0 ? options.speed : 1;
  const span = Math.max(0, end - start);
  let played = Math.max(0, t) * speed;
  if (options.direction === "boomerang" && Number.isFinite(span)) {
    played = played <= span ? played : Math.max(0, 2 * span - played);
  }
  const at =
    options.direction === "reverse" && Number.isFinite(end) ? end - played : start + played;
  return Math.min(Math.max(at, start), end);
}
