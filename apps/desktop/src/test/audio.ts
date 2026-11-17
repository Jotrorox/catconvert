/**
 * A stand-in for `HTMLAudioElement`, which jsdom can't play. Install it with
 * `installFakeAudio()`; URLs containing "broken" fail to load, and
 * `playable` decides what `canPlayType` answers.
 */
import { vi } from "vitest";

export class FakeAudio extends EventTarget {
  static all: FakeAudio[] = [];
  /** MIME types `canPlayType` says yes to; everything when null. */
  static playable: string[] | null = null;

  preload = "";
  crossOrigin: string | null = null;
  src = "";
  currentTime = 0;
  duration = Number.NaN;
  paused = true;
  ended = false;
  seeking = false;
  readyState = 0;

  constructor() {
    super();
    FakeAudio.all.push(this);
  }

  canPlayType(type: string): string {
    return FakeAudio.playable === null || FakeAudio.playable.includes(type) ? "maybe" : "";
  }

  removeAttribute(name: string) {
    if (name === "src") this.src = "";
    if (name === "crossorigin") this.crossOrigin = null;
  }

  load() {
    this.paused = true;
    this.readyState = 0;
    const src = this.src;
    if (!src) return;
    queueMicrotask(() => {
      if (this.src !== src) return;
      if (src.includes("broken")) {
        this.dispatchEvent(new Event("error"));
        return;
      }
      this.readyState = 4;
      this.duration = 60;
      this.dispatchEvent(new Event("durationchange"));
      this.dispatchEvent(new Event("loadedmetadata"));
    });
  }

  async play() {
    this.paused = false;
    this.ended = false;
    this.dispatchEvent(new Event("play"));
  }

  pause() {
    if (this.paused) return;
    this.paused = true;
    this.dispatchEvent(new Event("pause"));
  }

  /** The newest element that is playing. */
  static playing(): FakeAudio | undefined {
    return FakeAudio.all.filter((a) => !a.paused).at(-1);
  }
}

/** Animation frames queued by the code under test, run with `runFrames()`. */
let frames = new Map<number, FrameRequestCallback>();
let nextFrame = 1;

export function installFakeAudio() {
  FakeAudio.all = [];
  FakeAudio.playable = null;
  frames = new Map();
  vi.stubGlobal("Audio", FakeAudio);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(nextFrame, callback);
    return nextFrame++;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
}

/** Run the animation frames queued so far (each may queue the next). */
export function runFrames() {
  const due = [...frames.values()];
  frames = new Map();
  for (const callback of due) callback(performance.now());
}
