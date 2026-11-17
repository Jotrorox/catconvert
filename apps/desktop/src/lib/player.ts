/**
 * Sound playback for the editor, the Queue and History.
 *
 * Files play straight from disk through Tauri's asset protocol in an
 * `<audio>` element. When the webview can't decode one (WMA, AIFF or Opus in
 * some webviews, a video's AC-3 track…), the backend decodes the sound to a
 * temporary WAV file and that plays instead. Only one player sounds at a time,
 * and a player stops when its component goes away.
 *
 * The editor passes a `gain` so its sound runs through Web Audio, which lets
 * you hear the fades and loudness normalisation before converting.
 */
import { type Accessor, createEffect, createSignal, on, onCleanup } from "solid-js";
import * as api from "./api";

/** How long a file may take to load before the decoded copy is tried instead. */
const LOAD_TIMEOUT_MS = 6000;

/** Loudness `normalize` aims for, in LUFS (`LOUDNORM` in convert.rs). */
export const LOUDNESS_TARGET = -16;

/** Part of the file to play, read live so trim changes apply while playing. */
export type PlayRange = { start: number; end: number; loop: boolean };

export type PlayerOptions = {
  /**
   * MIME type (with codecs) of the sound, from {@link playbackType}. The
   * webview is asked whether it can play it; `null` skips straight to the
   * decoded copy. Leave it out when unknown: the file is simply tried.
   */
  type?: () => string | null;
  /**
   * Volume at a time in the file, 1 being unchanged, read on every frame
   * while playing. Setting it routes the sound through Web Audio.
   */
  gain?: (time: number) => number;
  /** Catch peaks above full scale, as `loudnorm` does, while true. */
  limit?: () => boolean;
};

export type Player = {
  /** Playing, or about to (loading). */
  playing: Accessor<boolean>;
  /** Position in seconds; follows playback on every frame. */
  time: Accessor<number>;
  /** Length of the sound, once loaded. */
  duration: Accessor<number | null>;
  /** Loading or decoding before the sound starts. */
  loading: Accessor<boolean>;
  /** Playing a range ("play selection") rather than the whole file. */
  inRange: Accessor<boolean>;
  /** Why the file couldn't be played. */
  error: Accessor<string | null>;
  /** The `gain` option is heard (false where Web Audio can't take the sound). */
  shapes: Accessor<boolean>;
  /** Play from `from` (default: where it is), within `range` if given. */
  play: (from?: number, range?: (() => PlayRange) | null) => void;
  pause: () => void;
  toggle: () => void;
  seek: (time: number) => void;
};

/** The player that is sounding right now, across the whole app. */
let sounding: { pause: () => void } | null = null;

const CONTAINERS: Record<string, string> = {
  mp3: "audio/mpeg",
  aac: "audio/aac",
  wav: "audio/wav",
  flac: "audio/flac",
  m4a: "audio/mp4",
  m4b: "audio/mp4",
  mp4: "audio/mp4",
  m4v: "audio/mp4",
  // MOV and MP4 share their structure; webviews that play one play both.
  mov: "audio/mp4",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  opus: "audio/ogg",
  webm: "audio/webm",
};

/** Containers that only ever hold one kind of sound. */
const SINGLE_CODEC = new Set(["audio/mpeg", "audio/aac", "audio/wav", "audio/flac"]);

/** ffprobe codec names to `codecs=` values, per container. */
const CODECS: Record<string, Record<string, string>> = {
  "audio/mp4": { aac: "mp4a.40.2", mp3: "mp4a.6B", alac: "alac", flac: "flac", opus: "opus" },
  "audio/ogg": { vorbis: "vorbis", opus: "opus", flac: "flac" },
  "audio/webm": { vorbis: "vorbis", opus: "opus" },
};

/**
 * The MIME type to ask the webview about for a file with this extension and
 * sound codec (ffprobe's name, `null` if unknown), or `null` when no webview
 * plays it and it has to be decoded first (WMA, AIFF, MKV, AC-3…).
 */
export function playbackType(extension: string | null, codec: string | null): string | null {
  const container = CONTAINERS[extension?.toLowerCase() ?? ""];
  if (!container) return null;
  if (SINGLE_CODEC.has(container) || codec === null) return container;
  const name = CODECS[container]?.[codec];
  return name ? `${container}; codecs="${name}"` : null;
}

/** The sound codec CatConvert writes for each audio format (convert.rs). */
const WRITTEN_CODEC: Partial<Record<api.OutputFormat, string>> = {
  mp3: "mp3",
  m4a: "aac",
  wav: "pcm_s16le",
  flac: "flac",
  ogg: "opus",
};

/** {@link playbackType} for a file CatConvert wrote in `format`. */
export function outputType(format: api.OutputFormat): string | null {
  return playbackType(format, WRITTEN_CODEC[format] ?? null);
}

/** Volume change that brings sound at `lufs` to {@link LOUDNESS_TARGET}, within ±20 dB. */
export function normalizeGain(lufs: number | null): number {
  if (lufs === null || !Number.isFinite(lufs)) return 1;
  return Math.min(10, Math.max(0.1, 10 ** ((LOUDNESS_TARGET - lufs) / 20)));
}

/** What the conversion does to the volume, for {@link soundGain}. */
export type SoundShape = {
  /** The kept range, in seconds of the file. */
  start: number;
  end: number;
  /** Fade lengths in seconds; 0 is off. */
  fadeIn: number;
  fadeOut: number;
  /** Loudness normalisation as a volume factor (1 when off). */
  level: number;
};

/**
 * Volume at `time` (seconds into the file) of the converted sound, relative
 * to the original: linear fades at the ends of the kept range, like ffmpeg's
 * `afade`, on top of the normalisation level.
 */
export function soundGain(time: number, shape: SoundShape): number {
  let gain = shape.level;
  if (time >= shape.start && time <= shape.end) {
    if (shape.fadeIn > 0) gain *= Math.min(1, Math.max(0, (time - shape.start) / shape.fadeIn));
    if (shape.fadeOut > 0) gain *= Math.min(1, Math.max(0, (shape.end - time) / shape.fadeOut));
  }
  return gain;
}

/** One way of loading the file: as is or decoded, with or without CORS. */
type Attempt = { decode: boolean; cors: boolean };

type Graph = { ctx: AudioContext; gain: GainNode; limiter: DynamicsCompressorNode };

/**
 * A player for the sound of `path()`. Nothing loads until the first `play`.
 * Must be called inside a component or root: it stops when that goes away.
 */
export function createPlayer(path: Accessor<string | null>, options: PlayerOptions = {}): Player {
  const [playing, setPlaying] = createSignal(false);
  const [time, setTime] = createSignal(0);
  const [duration, setDuration] = createSignal<number | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [inRange, setInRange] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [shapes, setShapes] = createSignal(options.gain !== undefined);

  let audio: HTMLAudioElement | null = null;
  let graph: Graph | null = null;
  /** Web Audio can't hear this source (no CORS), so it plays without the preview. */
  let plain = false;
  /** The path the element holds a playable source for. */
  let loadedFor: string | null = null;
  /** The loaded source is the decoded copy. */
  let decoded = false;
  let range: (() => PlayRange) | null = null;
  let wanted = false;
  /** Bumped by every play/pause, so a slow load can tell it was overtaken. */
  let request = 0;
  let loadToken = 0;
  let frame = 0;
  let alive = true;

  const handle = { pause: () => pause() };

  const element = (): HTMLAudioElement => {
    if (audio) return audio;
    const el = new Audio();
    el.preload = "auto";
    el.addEventListener("durationchange", () => {
      setDuration(Number.isFinite(el.duration) ? el.duration : null);
    });
    el.addEventListener("ended", () => {
      const r = range?.();
      if (wanted && r?.loop) {
        el.currentTime = r.start;
        void el.play().catch(() => pause());
        return;
      }
      pause();
      if (r) seek(r.start);
      else setTime(el.duration || time());
    });
    // Media keys, AirPods taken out, another app grabbing the sound…
    el.addEventListener("pause", () => {
      if (wanted && !el.ended && el.readyState > 0 && !el.seeking) pause();
    });
    audio = el;
    return el;
  };

  /** Point the element at `url`; resolves to whether it can be played. */
  const tryLoad = (el: HTMLAudioElement, url: string, cors: boolean) =>
    new Promise<boolean>((resolve) => {
      const done = (ok: boolean) => {
        clearTimeout(timer);
        el.removeEventListener("loadedmetadata", yes);
        el.removeEventListener("error", no);
        resolve(ok);
      };
      const yes = () => done(true);
      const no = () => done(false);
      const timer = setTimeout(no, LOAD_TIMEOUT_MS);
      el.addEventListener("loadedmetadata", yes);
      el.addEventListener("error", no);
      if (cors) el.crossOrigin = "anonymous";
      else el.removeAttribute("crossorigin");
      el.src = url;
      el.load();
    });

  /** Make sure the element holds a playable source for `p`. */
  const load = async (p: string): Promise<boolean> => {
    if (loadedFor === p) return true;
    const mine = ++loadToken;
    const el = element();
    const wantsCors = options.gain !== undefined;
    const type = options.type ? options.type() : undefined;
    const native = type === undefined || (type !== null && el.canPlayType(type) !== "");
    // Web Audio needs CORS. Should a webview refuse it, the same source
    // loading without it tells: play that, without the sound preview.
    const ways = (decode: boolean): Attempt[] =>
      wantsCors
        ? [
            { decode, cors: true },
            { decode, cors: false },
          ]
        : [{ decode, cors: false }];
    const attempts = [...(native ? ways(false) : []), ...ways(true)];
    setLoading(true);
    setError(null);
    try {
      for (const attempt of attempts) {
        const url = await api.playbackUrl(p, attempt.decode);
        if (mine !== loadToken || !alive) return false;
        const ok = await tryLoad(el, url, attempt.cors);
        if (mine !== loadToken || !alive) return false;
        if (ok) {
          loadedFor = p;
          decoded = attempt.decode;
          plain = !attempt.cors;
          if (plain) setShapes(false);
          return true;
        }
      }
      setError("This sound can't be played.");
      return false;
    } catch (e) {
      if (mine === loadToken) setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally {
      if (mine === loadToken) setLoading(false);
    }
  };

  const connect = (el: HTMLAudioElement) => {
    if (!options.gain || graph || plain) return;
    const Context =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!Context) {
      setShapes(false);
      return;
    }
    try {
      const ctx = new Context();
      const source = ctx.createMediaElementSource(el);
      const gain = ctx.createGain();
      const limiter = ctx.createDynamicsCompressor();
      limiter.threshold.value = -1.5;
      limiter.knee.value = 0;
      limiter.ratio.value = 1;
      limiter.attack.value = 0.002;
      limiter.release.value = 0.15;
      source.connect(gain).connect(limiter).connect(ctx.destination);
      graph = { ctx, gain, limiter };
    } catch {
      setShapes(false);
    }
  };

  /** Follow the sound settings at `t`; `now` jumps there instead of gliding. */
  const shape = (t: number, now = false) => {
    if (!graph || !options.gain) return;
    const value = Math.max(0, options.gain(t));
    if (now) graph.gain.gain.value = value;
    else graph.gain.gain.setTargetAtTime(value, graph.ctx.currentTime, 0.015);
    graph.limiter.ratio.value = options.limit?.() ? 20 : 1;
  };

  const tick = () => {
    cancelAnimationFrame(frame);
    if (!audio || !wanted) return;
    const t = audio.currentTime;
    const r = range?.();
    if (r && t >= r.end && !audio.seeking) {
      if (r.loop) {
        audio.currentTime = r.start;
        setTime(r.start);
        shape(r.start, true);
      } else {
        pause();
        seek(r.start);
        return;
      }
    } else {
      setTime(t);
      shape(t);
    }
    frame = requestAnimationFrame(tick);
  };

  const play = (from?: number, within: (() => PlayRange) | null = null) => {
    const p = path();
    if (!p) return;
    range = within;
    setInRange(within !== null);
    let at = from ?? time();
    const length = duration();
    if (!within && length !== null && at >= length - 0.05) at = 0;
    setTime(at);
    if (sounding && sounding !== handle) sounding.pause();
    sounding = handle;
    setError(null);
    wanted = true;
    setPlaying(true);
    const mine = ++request;
    void (async () => {
      if (!(await load(p))) {
        if (mine === request) {
          wanted = false;
          setPlaying(false);
        }
        return;
      }
      if (mine !== request || !audio) return;
      const el = audio;
      connect(el);
      el.currentTime = time();
      shape(time(), true);
      try {
        await graph?.ctx.resume();
        await el.play();
      } catch (e) {
        if (mine !== request) return;
        wanted = false;
        setPlaying(false);
        if (!(e instanceof DOMException && e.name === "AbortError")) {
          setError(decoded ? "This sound can't be played." : String(e));
        }
        return;
      }
      if (mine === request) tick();
    })();
  };

  const pause = () => {
    request++;
    wanted = false;
    cancelAnimationFrame(frame);
    if (audio && loadedFor) {
      audio.pause();
      setTime(audio.currentTime);
    }
    setPlaying(false);
    if (sounding === handle) sounding = null;
  };

  const seek = (t: number) => {
    const length = duration();
    const at = Math.max(0, length !== null ? Math.min(t, length) : t);
    const r = range?.();
    // Seeking out of the selection plays on normally from there.
    if (r && (at < r.start || at > r.end)) {
      range = null;
      setInRange(false);
    }
    setTime(at);
    if (audio && loadedFor) {
      audio.currentTime = at;
      shape(at, true);
    }
  };

  const unload = () => {
    pause();
    loadToken++;
    loadedFor = null;
    decoded = false;
    setDuration(null);
    setError(null);
    setLoading(false);
    if (audio) {
      audio.removeAttribute("src");
      audio.load();
    }
    // An element wired into Web Audio stays wired; start afresh next time.
    if (graph) {
      void graph.ctx.close().catch(() => {});
      graph = null;
      audio = null;
    }
  };

  createEffect(
    on(
      path,
      () => {
        unload();
        setTime(0);
      },
      { defer: true },
    ),
  );

  onCleanup(() => {
    alive = false;
    unload();
  });

  return {
    playing,
    time,
    duration,
    loading,
    inRange,
    error,
    shapes,
    play,
    pause,
    toggle: () => (playing() ? pause() : play()),
    seek,
  };
}
