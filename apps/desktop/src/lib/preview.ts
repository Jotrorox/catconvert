/**
 * Reactive helpers that turn ffmpeg frame grabs and waveforms into image URLs.
 * Object URLs are revoked when replaced or when the owning component unmounts.
 */
import { type Accessor, createEffect, createSignal, on, onCleanup } from "solid-js";
import * as api from "./api";

function createUrlSlot() {
  const [url, setUrl] = createSignal<string | null>(null);
  const set = (next: string | null) => {
    const previous = url();
    setUrl(next);
    if (previous) URL.revokeObjectURL(previous);
  };
  onCleanup(() => set(null));
  return [url, set] as const;
}

/**
 * A frame of `path` at `time()` seconds, tone-mapped when `hdr` is set. While
 * a grab is running, newer times replace the pending one, so dragging a handle
 * never piles up ffmpeg runs.
 */
export function createFrame(
  path: Accessor<string | null>,
  time: Accessor<number>,
  height: number,
  hdr = false,
): Accessor<string | null> {
  const [url, setUrl] = createUrlSlot();
  let busy = false;
  let wanted: { path: string; time: number } | null = null;
  let alive = true;
  onCleanup(() => {
    alive = false;
  });

  const pump = async () => {
    if (busy) return;
    busy = true;
    while (wanted && alive) {
      const job = wanted;
      wanted = null;
      try {
        const next = await api.previewFrame(job.path, job.time, height, hdr);
        if (alive) setUrl(next);
        else URL.revokeObjectURL(next);
      } catch {
        // Keep showing the previous frame; a missing frame is not worth an error.
      }
    }
    busy = false;
  };

  createEffect(() => {
    const p = path();
    if (!p) return;
    // Milliseconds, as ffmpeg is given them: stepping one frame must show it.
    wanted = { path: p, time: Math.max(0, Math.round(time() * 1000) / 1000) };
    void pump();
  });
  return url;
}

/** `count` evenly spaced thumbnails across a clip, loaded a few at a time. */
export function createFilmstrip(
  path: Accessor<string | null>,
  duration: Accessor<number | null>,
  count: number,
  height: number,
  hdr = false,
): Accessor<(string | null)[]> {
  const [frames, setFrames] = createSignal<(string | null)[]>([]);
  let generation = 0;
  const revokeAll = () => {
    for (const url of frames()) if (url) URL.revokeObjectURL(url);
  };
  onCleanup(() => {
    generation++;
    revokeAll();
  });

  createEffect(
    on([path, duration], ([p, d]) => {
      const mine = ++generation;
      revokeAll();
      setFrames(Array.from({ length: count }, () => null));
      if (!p || !d) return;
      const times = Array.from({ length: count }, (_, i) => ((i + 0.5) / count) * d);
      const load = async (i: number) => {
        const t = times[i];
        if (t === undefined) return;
        try {
          const url = await api.previewFrame(p, t, height, hdr);
          if (mine !== generation) {
            URL.revokeObjectURL(url);
            return;
          }
          setFrames((list) => list.map((old, j) => (j === i ? url : old)));
        } catch {
          // Leave the slot empty.
        }
      };
      void (async () => {
        for (let i = 0; i < count && mine === generation; i += 3) {
          await Promise.all([load(i), load(i + 1), load(i + 2)]);
        }
      })();
    }),
  );
  return frames;
}

/** The waveform of `path`'s audio, drawn in `color`. */
export function createWaveform(
  path: Accessor<string | null>,
  color: string,
  width = 1200,
  height = 160,
): Accessor<string | null> {
  const [url, setUrl] = createUrlSlot();
  let generation = 0;
  onCleanup(() => {
    generation++;
  });
  createEffect(
    on(path, (p) => {
      const mine = ++generation;
      setUrl(null);
      if (!p) return;
      api.waveform(p, width, height, color).then(
        (next) => {
          if (mine === generation) setUrl(next);
          else URL.revokeObjectURL(next);
        },
        () => {},
      );
    }),
  );
  return url;
}

/**
 * One small frame per picture, keyed by path so reordering never refetches.
 * Loaded a few at a time; read with the returned lookup.
 */
export function createThumbnails(
  paths: Accessor<readonly string[]>,
  height: number,
): (path: string) => string | null {
  const [urls, setUrls] = createSignal<Readonly<Record<string, string>>>({});
  const requested = new Set<string>();
  const waiting: string[] = [];
  let running = 0;
  let alive = true;
  onCleanup(() => {
    alive = false;
    for (const url of Object.values(urls())) URL.revokeObjectURL(url);
  });

  const pump = () => {
    while (alive && running < 3) {
      const path = waiting.shift();
      if (path === undefined) return;
      running++;
      api
        .previewFrame(path, 0, height)
        .then(
          (url) => {
            if (alive) setUrls((all) => ({ ...all, [path]: url }));
            else URL.revokeObjectURL(url);
          },
          () => {
            // Leave the tile blank; the backend reports real problems on convert.
          },
        )
        .finally(() => {
          running--;
          pump();
        });
    }
  };

  createEffect(() => {
    for (const path of paths()) {
      if (requested.has(path)) continue;
      requested.add(path);
      waiting.push(path);
    }
    pump();
  });
  return (path) => urls()[path] ?? null;
}
