import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FakeAudio, installFakeAudio, runFrames } from "../test/audio";
import * as api from "./api";
import {
  createPlayer,
  normalizeGain,
  outputType,
  type Player,
  type PlayerOptions,
  playbackType,
  soundGain,
} from "./player";

vi.mock("./api", () => ({
  playbackUrl: vi.fn(async (path: string, decode: boolean) =>
    decode ? "asset://localhost/tmp/decoded.wav" : `asset://localhost${path}`,
  ),
}));

/** Let loads (microtasks and resolved promises) settle. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

let disposers: (() => void)[] = [];

function player(path: string | null, options?: PlayerOptions): Player {
  return createRoot((dispose) => {
    disposers.push(dispose);
    return createPlayer(() => path, options);
  });
}

beforeEach(() => {
  installFakeAudio();
  vi.clearAllMocks();
});

afterEach(() => {
  for (const dispose of disposers) dispose();
  disposers = [];
  vi.unstubAllGlobals();
});

describe("playbackType", () => {
  it("names the container and codec for the webview to judge", () => {
    expect(playbackType("mp3", "mp3")).toBe("audio/mpeg");
    expect(playbackType("WAV", "pcm_s24le")).toBe("audio/wav");
    expect(playbackType("m4a", "aac")).toBe('audio/mp4; codecs="mp4a.40.2"');
    expect(playbackType("mov", "aac")).toBe('audio/mp4; codecs="mp4a.40.2"');
    expect(playbackType("m4a", "alac")).toBe('audio/mp4; codecs="alac"');
    expect(playbackType("ogg", "vorbis")).toBe('audio/ogg; codecs="vorbis"');
    expect(playbackType("opus", "opus")).toBe('audio/ogg; codecs="opus"');
    expect(playbackType("webm", null)).toBe("audio/webm");
  });

  it("sends what no webview plays straight to decoding", () => {
    expect(playbackType("wma", "wmav2")).toBeNull();
    expect(playbackType("aiff", "pcm_s16be")).toBeNull();
    expect(playbackType("mkv", "aac")).toBeNull();
    expect(playbackType("mp4", "ac3")).toBeNull();
    expect(playbackType(null, "aac")).toBeNull();
  });

  it("knows the codecs CatConvert writes", () => {
    expect(outputType("ogg")).toBe('audio/ogg; codecs="opus"');
    expect(outputType("m4a")).toBe('audio/mp4; codecs="mp4a.40.2"');
    expect(outputType("flac")).toBe("audio/flac");
  });
});

describe("sound preview", () => {
  it("brings loudness to -16 LUFS within ±20 dB", () => {
    expect(normalizeGain(-16)).toBeCloseTo(1);
    expect(normalizeGain(-22)).toBeCloseTo(1.995, 2);
    expect(normalizeGain(-60)).toBe(10);
    expect(normalizeGain(10)).toBe(0.1);
    expect(normalizeGain(null)).toBe(1);
  });

  it("fades linearly at the ends of the kept range", () => {
    const shape = { start: 10, end: 20, fadeIn: 2, fadeOut: 4, level: 1 };
    expect(soundGain(10, shape)).toBe(0);
    expect(soundGain(11, shape)).toBeCloseTo(0.5);
    expect(soundGain(15, shape)).toBe(1);
    expect(soundGain(19, shape)).toBeCloseTo(0.25);
    expect(soundGain(20, shape)).toBe(0);
    // Outside the kept range nothing fades; the level applies everywhere.
    expect(soundGain(5, { ...shape, level: 2 })).toBe(2);
    expect(soundGain(12, { ...shape, level: 2 })).toBe(2);
  });
});

describe("createPlayer", () => {
  it("loads nothing until played, then plays the file as it is", async () => {
    const p = player("/music/purr.mp3", { type: () => "audio/mpeg" });
    await settle();
    expect(api.playbackUrl).not.toHaveBeenCalled();

    p.play();
    expect(p.playing()).toBe(true);
    await settle();
    expect(api.playbackUrl).toHaveBeenCalledWith("/music/purr.mp3", false);
    const audio = FakeAudio.playing();
    expect(audio?.src).toBe("asset://localhost/music/purr.mp3");
    expect(p.duration()).toBe(60);

    if (audio) audio.currentTime = 12.5;
    runFrames();
    expect(p.time()).toBe(12.5);
    p.pause();
    expect(p.playing()).toBe(false);
    expect(audio?.paused).toBe(true);
  });

  it("plays a decoded copy when the webview can't play the file", async () => {
    const p = player("/music/broken.flac");
    p.play();
    await settle();
    await settle();
    expect(vi.mocked(api.playbackUrl).mock.calls).toEqual([
      ["/music/broken.flac", false],
      ["/music/broken.flac", true],
    ]);
    expect(FakeAudio.playing()?.src).toContain("decoded.wav");
    expect(p.error()).toBeNull();
  });

  it("plays without the sound preview where Web Audio can't hear the file", async () => {
    const p = player("/music/purr.mp3", { gain: () => 1 });
    const load = FakeAudio.prototype.load;
    // A webview that refuses CORS for the asset protocol.
    vi.spyOn(FakeAudio.prototype, "load").mockImplementation(function (this: FakeAudio) {
      if (this.crossOrigin) this.src = `${this.src}#broken`;
      load.call(this);
    });
    p.play();
    await settle();
    await settle();
    expect(vi.mocked(api.playbackUrl).mock.calls).toEqual([
      ["/music/purr.mp3", false],
      ["/music/purr.mp3", false],
    ]);
    const audio = FakeAudio.playing();
    expect(audio?.crossOrigin).toBeNull();
    expect(audio?.src).toBe("asset://localhost/music/purr.mp3");
    expect(p.shapes()).toBe(false);
  });

  it("decodes right away what the webview says it can't play", async () => {
    FakeAudio.playable = ["audio/mpeg"];
    const p = player("/music/old.m4a", { type: () => 'audio/mp4; codecs="alac"' });
    p.play();
    await settle();
    expect(vi.mocked(api.playbackUrl).mock.calls).toEqual([["/music/old.m4a", true]]);
  });

  it("says when the sound can't be played at all", async () => {
    vi.mocked(api.playbackUrl).mockRejectedValueOnce(new Error("ffmpeg was not found"));
    const p = player("/music/purr.mp3", { type: () => null });
    p.play();
    await settle();
    expect(p.playing()).toBe(false);
    expect(p.error()).toBe("ffmpeg was not found");
  });

  it("lets only one player sound at a time", async () => {
    const a = player("/music/a.mp3");
    const b = player("/music/b.mp3");
    a.play();
    await settle();
    b.play();
    await settle();
    expect(a.playing()).toBe(false);
    expect(b.playing()).toBe(true);
    expect(FakeAudio.all.filter((audio) => !audio.paused)).toHaveLength(1);
  });

  it("plays a selection, loops it if asked and stops at its end otherwise", async () => {
    const [loop, setLoop] = createSignal(true);
    const p = player("/music/purr.mp3");
    p.play(10, () => ({ start: 10, end: 20, loop: loop() }));
    await settle();
    const audio = FakeAudio.playing() as FakeAudio;
    expect(audio.currentTime).toBe(10);
    expect(p.inRange()).toBe(true);

    audio.currentTime = 20.01;
    runFrames();
    expect(audio.currentTime).toBe(10);
    expect(p.playing()).toBe(true);

    setLoop(false);
    audio.currentTime = 20.01;
    runFrames();
    expect(p.playing()).toBe(false);
    expect(p.time()).toBe(10);
  });

  it("drops out of the selection when seeking past it", async () => {
    const p = player("/music/purr.mp3");
    p.play(10, () => ({ start: 10, end: 20, loop: true }));
    await settle();
    p.seek(30);
    expect(p.inRange()).toBe(false);
    expect(FakeAudio.playing()?.currentTime).toBe(30);
  });

  it("starts over when played again from the end", async () => {
    const p = player("/music/purr.mp3");
    p.play();
    await settle();
    p.pause();
    p.seek(60);
    p.play();
    await settle();
    expect(FakeAudio.playing()?.currentTime).toBe(0);
  });

  it("stops and lets go of the file when its owner goes away", async () => {
    const p = player("/music/purr.mp3");
    p.play();
    await settle();
    const audio = FakeAudio.playing() as FakeAudio;
    for (const dispose of disposers) dispose();
    disposers = [];
    expect(audio.paused).toBe(true);
    expect(audio.src).toBe("");
  });
});
