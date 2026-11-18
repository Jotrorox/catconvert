import { fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type {
  ConvertOptions,
  FfmpegStatus,
  MediaInfo,
  OutputFormat,
  UpscalerDownload,
  UpscalerInfo,
} from "./lib/api";
import * as api from "./lib/api";
import { DEFAULTS } from "./lib/formats";
import {
  aiModels,
  applyProgress,
  navigate,
  openPaths,
  resetStore,
  setTheme,
  state,
  updateSettings,
} from "./lib/store";
import { FakeAudio, installFakeAudio } from "./test/audio";

vi.mock("./lib/api", () => ({
  inTauri: false,
  ffmpegStatus: vi.fn(),
  relocateFfmpeg: vi.fn(),
  probeFile: vi.fn(),
  previewFrame: vi.fn(async () => "blob:frame"),
  compareSample: vi.fn(),
  compareFile: vi.fn(),
  waveform: vi.fn(async () => "blob:wave"),
  playbackUrl: vi.fn(async (path: string) => `asset://localhost${path}`),
  loudness: vi.fn(async () => -22),
  previewCommand: vi.fn(async () => "ffmpeg -i clip.mov clip.mp4"),
  // The core's numbers for H.264 with AAC: CRF 34 → 18, AAC 96/128/192 kbps.
  qualitySummary: vi.fn(async (info: MediaInfo, options: ConvertOptions) => {
    if (options.format === "gif") return { audio: null, video: null };
    const q = options.quality;
    const kbps = q < 34 ? 96 : q < 67 ? 128 : 192;
    const hardware = { hardware: false, hardwareQuality: null };
    return {
      audio: info.audioCodec ? { mode: "bitrate", kbps } : null,
      video: options.hardware
        ? { codec: "h265", crf: null, kbps: null, passes: 1, hardware: true, hardwareQuality: 64 }
        : options.videoBitrate
          ? { codec: "h264", crf: null, kbps: options.videoBitrate, passes: 2, ...hardware }
          : {
              codec: "h264",
              crf: Math.round(34 - (16 * q) / 100),
              kbps: null,
              passes: 1,
              ...hardware,
            },
    };
  }),
  previewSequenceCommand: vi.fn(async () => "ffmpeg -f concat -i list.txt cat.gif"),
  outputPreview: vi.fn(),
  sequenceOutputPreview: vi.fn(async (_pictures: MediaInfo[], options: ConvertOptions) => ({
    path: `/photos/cat.${options.format}`,
    exists: false,
  })),
  chooseSavePath: vi.fn(async () => null),
  askReplace: vi.fn(async () => "cancel"),
  saveFrame: vi.fn(),
  checkCommand: vi.fn(async (_info: unknown, _options: unknown, text: string) => text),
  fillCommand: vi.fn(),
  commandTemplate: vi.fn(),
  startCustomJob: vi.fn(),
  startJob: vi.fn(),
  startSequence: vi.fn(),
  cancelJob: vi.fn(async () => true),
  pickFiles: vi.fn(async () => []),
  pickFolder: vi.fn(async () => null),
  defaultOutputFolder: vi.fn(async () => "/Users/cat/Downloads/CatConvert"),
  prepareOutputFolder: vi.fn(async (path: string) => path),
  setWindowTheme: vi.fn(async () => {}),
  thirdPartyLicenses: vi.fn(async () => "ffmpeg 9.0.2 (GPL-2.0-or-later)"),
  revealInFolder: vi.fn(async () => {}),
  missingFiles: vi.fn(async () => []),
  onJobProgress: vi.fn(),
  onFileDrop: vi.fn(),
  appVersion: vi.fn(async () => "0.1.2"),
  checkForUpdate: vi.fn(),
  installUpdate: vi.fn(),
  relaunch: vi.fn(async () => {}),
  onOpenedFiles: vi.fn(),
  quickActionStatus: vi.fn(async () => ({ supported: false, installed: false })),
  setQuickAction: vi.fn(),
  dragFiles: vi.fn(async () => {}),
  upscalerStatus: vi.fn(async () => null),
  checkUpscalerGpu: vi.fn(),
  downloadUpscaler: vi.fn(),
  cancelUpscalerDownload: vi.fn(async () => null),
  pickUpscalerFiles: vi.fn(async () => []),
  installUpscalerFiles: vi.fn(),
  removeUpscaler: vi.fn(),
  aiEstimate: vi.fn(async () => [2, 9]),
  NO_JXL: {},
  NEXT_TO_ORIGINAL: { folder: null, name: null, collision: "keepBoth" },
  NAME_PLACEHOLDERS: [
    { token: "{name}", meaning: "the original's name" },
    { token: "{format}", meaning: "the new extension" },
  ],
}));

/** Next to the original under the usual name, as the backend gets it by default. */
const USUAL = { folder: null, name: null, collision: "keepBoth" };

const ALL: OutputFormat[] = [
  ...["mp4", "webm", "mov", "mkv", "gif"],
  ...["mp3", "m4a", "wav", "flac", "ogg"],
  ...["png", "jpg", "webp", "avif", "jxl"],
] as OutputFormat[];

function status(found = true): FfmpegStatus {
  return {
    found,
    bundled: false,
    version: found ? "7.1" : null,
    path: found ? "/opt/homebrew/bin/ffmpeg" : null,
    error: found ? null : "ffmpeg was not found",
    formats: ALL.map((format) => ({
      format,
      available: found && format !== "webp",
      animated: found && ["mp4", "webm", "mov", "mkv", "gif", "jxl"].includes(format),
    })),
    videoCodecs: found ? ["h264", "h265", "vp9", "av1"] : [],
    hevcHardware: false,
    readsAv1: found,
    libjxl: found ? "0.13.0" : null,
    jxl: {
      cjxl: found,
      djxl: found,
      ffmpegEncoder: false,
      ffmpegAnimEncoder: false,
      ffmpegDecoder: false,
      ffmpegAnimDecoder: false,
    },
  };
}

const VIDEO: MediaInfo = {
  path: "/clips/clip.mov",
  fileName: "clip.mov",
  extension: "mov",
  sizeBytes: 48_000_000,
  kind: "video",
  durationSecs: 42,
  width: 1920,
  height: 1080,
  fps: 30,
  videoCodec: "h264",
  audioCodec: "aac",
  sampleRate: 48000,
  audioChannels: 2,
  bitrate: 9_142_857,
  videoBitrate: 9_014_857,
  audioBitrate: 128_000,
  reconstructsJpeg: false,
  hdr: false,
  hasLocation: false,
  locationTags: [],
  tileGrid: null,
};

const PHOTO: MediaInfo = {
  ...VIDEO,
  path: "/photos/cat.jpg",
  fileName: "cat.jpg",
  extension: "jpg",
  kind: "image",
  durationSecs: null,
  width: 4032,
  height: 3024,
  fps: null,
  videoCodec: "mjpeg",
  audioCodec: null,
  sampleRate: null,
};

const PACKED: MediaInfo = {
  ...PHOTO,
  path: "/photos/cat.jxl",
  fileName: "cat.jxl",
  extension: "jxl",
  videoCodec: "jpegxl",
  reconstructsJpeg: true,
  tileGrid: null,
};

const SONG: MediaInfo = {
  ...VIDEO,
  path: "/music/purr.wav",
  fileName: "purr.wav",
  extension: "wav",
  kind: "audio",
  width: null,
  height: null,
  fps: null,
  videoCodec: null,
  audioCodec: "pcm_s16le",
  sampleRate: 48000,
};

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
  resetStore();
  vi.clearAllMocks();
  installFakeAudio();
  vi.mocked(api.ffmpegStatus).mockResolvedValue(status());
  vi.mocked(api.quickActionStatus).mockResolvedValue({ supported: false, installed: false });
  vi.mocked(api.probeFile).mockImplementation(async (path) =>
    path.endsWith(".wav")
      ? SONG
      : path.endsWith(".jpg")
        ? PHOTO
        : path.endsWith(".jxl")
          ? PACKED
          : { ...VIDEO, path, fileName: path.split("/").pop() ?? path },
  );
  vi.mocked(api.outputPreview).mockImplementation(async (info, options) => ({
    path: info.path.replace(/\.\w+$/, `.${options.format}`),
    exists: false,
  }));
  vi.mocked(api.chooseSavePath).mockResolvedValue(null);
  vi.mocked(api.pickFolder).mockResolvedValue(null);
  vi.mocked(api.askReplace).mockResolvedValue("cancel");
  let samples = 0;
  vi.mocked(api.compareSample).mockImplementation(async (_info, _options, side, _at, limit) => ({
    url: `asset://localhost/samples/${side}-${++samples}.png`,
    bytes: side === "converted" ? 612_000 : null,
    exact: side === "converted" && limit === null,
  }));
  vi.mocked(api.compareFile).mockImplementation(async () => ({
    url: `asset://localhost/samples/file-${++samples}.png`,
    bytes: 0,
    exact: true,
  }));
  vi.mocked(api.startJob).mockImplementation(async (path, options) => ({
    id: 7,
    output: path.replace(/\.\w+$/, `.${options.format}`),
    inputs: [VIDEO],
    command: "ffmpeg …",
  }));
});

function ffmpegFound() {
  return waitFor(() => expect(state.ffmpeg?.found).toBe(true));
}

async function openSong() {
  render(() => <App />);
  await ffmpegFound();
  openPaths([SONG.path]);
  return screen.findByRole("heading", { name: "purr.wav" });
}

/** Let the player load (microtasks and resolved promises). */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

async function openPhoto() {
  render(() => <App />);
  await ffmpegFound();
  openPaths([PHOTO.path]);
  return screen.findByRole("heading", { name: "cat.jpg" });
}

/** The calls to `compareSample` for one half. */
function sampleCalls(side: api.SampleSide) {
  return vi.mocked(api.compareSample).mock.calls.filter((call) => call[2] === side);
}

async function openVideo() {
  render(() => <App />);
  await ffmpegFound();
  openPaths([VIDEO.path]);
  return screen.findByRole("heading", { name: "clip.mov" });
}

describe("CatConvert", () => {
  it("starts on Convert once it has found ffmpeg", async () => {
    render(() => <App />);
    expect(screen.getByRole("heading", { name: "Drop files on the kitty" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Convert" })).toHaveAttribute("aria-current", "page");
    await ffmpegFound();
    expect(screen.queryByText("ffmpeg is missing.")).not.toBeInTheDocument();
    expect(screen.getByText(/Nothing yet/)).toBeInTheDocument();
  });

  it("warns when ffmpeg is missing and blocks converting", async () => {
    vi.mocked(api.ffmpegStatus).mockResolvedValue(status(false));
    render(() => <App />);
    expect(await screen.findByText("ffmpeg is missing.")).toBeInTheDocument();
    openPaths([VIDEO.path]);
    await screen.findByRole("heading", { name: "clip.mov" });
    expect(screen.getByRole("button", { name: /Start converting/ })).toBeDisabled();
  });

  it("shows the bundled ffmpeg and its licenses in Settings", async () => {
    vi.mocked(api.ffmpegStatus).mockResolvedValue({ ...status(), bundled: true });
    render(() => <App />);
    navigate("settings");
    expect(await screen.findByText("Built into CatConvert")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show licenses" }));
    expect(await screen.findByText("ffmpeg 9.0.2 (GPL-2.0-or-later)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide licenses" })).toBeInTheDocument();
  });

  it("steps through frames and saves the one on screen right away", async () => {
    vi.mocked(api.saveFrame).mockResolvedValue({
      output: "/clips/clip-frame-0m01s.png",
      outputBytes: 2_100_000,
    });
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: "Rotate 90°" }));

    // The preview opens at 1 s; at 30 fps that's frame 30. Net one frame on.
    fireEvent.click(screen.getByRole("button", { name: "Next frame" }));
    fireEvent.keyDown(window, { key: "ArrowRight" });
    fireEvent.keyDown(window, { key: "," });
    fireEvent.click(screen.getByRole("button", { name: "Save this frame" }));

    expect(await screen.findByText("Saved frame")).toBeInTheDocument();
    const [path, options, at] = vi.mocked(api.saveFrame).mock.calls[0] ?? [];
    expect(path).toBe(VIDEO.path);
    expect(options).toMatchObject({ rotate: 90, format: "mp4" });
    expect(at).toBeCloseTo(31 / 30, 2);
    expect(Math.ceil((at ?? 0) * 30)).toBe(31);
    expect(api.startJob).not.toHaveBeenCalled();
    expect(state.history[0]).toMatchObject({ name: "clip-frame-0m01s.png", format: "png" });

    fireEvent.click(screen.getByRole("button", { name: "Show in folder" }));
    expect(api.revealInFolder).toHaveBeenCalledWith("/clips/clip-frame-0m01s.png");

    // S saves too, except while typing in a field.
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Start" }), { key: "s" });
    expect(api.saveFrame).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(window, { key: "s" });
    await waitFor(() => expect(api.saveFrame).toHaveBeenCalledTimes(2));
  });

  it("says why a frame couldn't be saved", async () => {
    vi.mocked(api.saveFrame).mockRejectedValue("no frame at that time");
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: "Save this frame" }));
    expect(
      await screen.findByText("Couldn't save the frame: No frame at that time"),
    ).toBeInTheDocument();
    expect(state.history).toEqual([]);
  });

  it("cleans up the sound and draws the fades on the trim bar", async () => {
    await openVideo();
    expect(screen.getByRole("heading", { name: "Sound" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("switch", { name: "Even out loudness" }));
    const fadeIn = within(screen.getByRole("group", { name: "Fade in" }));
    const fadeOut = within(screen.getByRole("group", { name: "Fade out" }));
    expect(fadeIn.getByRole("button", { name: "Off" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(fadeIn.getByRole("button", { name: "1 s" }));
    fireEvent.click(fadeOut.getByRole("button", { name: "3 s" }));
    expect(document.querySelectorAll(".trim-fades .fade-line")).toHaveLength(2);
    expect(screen.getByText(/picture fades from and to black/)).toBeInTheDocument();

    const changes = within(screen.getByRole("list", { name: "Changes" }));
    expect(changes.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Even loudness",
      "Fades in 1 s",
      "Fades out 3 s",
    ]);

    // Without sound there is nothing to clean up.
    fireEvent.click(screen.getByRole("switch", { name: "Keep sound" }));
    expect(screen.queryByRole("heading", { name: "Sound" })).not.toBeInTheDocument();
    expect(document.querySelector(".trim-fades")).toBeNull();
    fireEvent.click(screen.getByRole("switch", { name: "Keep sound" }));

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ sound: { normalize: true, fadeIn: 1, fadeOut: 3 } }),
      USUAL,
    );
  });

  it("edits a video and queues it with the chosen settings", async () => {
    await openVideo();
    expect(screen.getByText(/1920×1080 · 0:42 · H264/)).toBeInTheDocument();
    // Video input offers video and sound formats, not pictures; MP4 is the default.
    expect(screen.getByRole("button", { name: "MP4" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: "PNG" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "WEBM" }));
    expect(screen.getByRole("button", { name: "WEBM" })).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(screen.getByRole("switch", { name: "Keep sound" }));
    fireEvent.click(screen.getByRole("button", { name: "Rotate 90°" }));
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "720" } });

    const start = screen.getByRole("slider", { name: "Trim start" });
    fireEvent.keyDown(start, { key: "ArrowRight", shiftKey: true });
    fireEvent.keyDown(start, { key: "ArrowRight", shiftKey: true });

    const changes = within(screen.getByRole("list", { name: "Changes" }));
    expect(changes.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Keeps 40 s",
      "720p",
      "Turned 90°",
      "No sound",
    ]);
    expect(await screen.findByText("Saved as clip.webm next to the original.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "File name" })).toHaveAttribute(
      "placeholder",
      "clip",
    );

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      {
        format: "webm",
        trimStart: 2,
        trimEnd: null,
        maxSize: 720,
        upscale: null,
        scaler: "auto",
        ai: null,
        fps: null,
        rotate: 90,
        flip: false,
        crop: null,
        quality: 60,
        keepAudio: false,
        videoCodec: null,
        videoBitrate: null,
        hardware: false,
        jxl: { lossless: false, effort: 7, progressive: false },
        sound: { normalize: false, fadeIn: 0, fadeOut: 0 },
        metadata: "noLocation",
        speed: 1,
        direction: "forward",
        destination: { kind: "nextToInput" },
      },
      USUAL,
    );

    applyProgress({
      jobId: 7,
      stage: "running",
      progress: 0.42,
      speed: 2,
      outputBytes: null,
      error: null,
    });
    expect(await screen.findByRole("progressbar")).toHaveAttribute("aria-valuenow", "42");
    expect(screen.getByText("2.0×")).toBeInTheDocument();

    applyProgress({
      jobId: 7,
      stage: "done",
      progress: 1,
      speed: null,
      outputBytes: 3_000_000,
      error: null,
    });
    const done = await screen.findByRole("button", { name: /Show in folder/ });
    fireEvent.click(done);
    expect(api.revealInFolder).toHaveBeenCalledWith("/clips/clip.webm");
    expect(screen.getByText("All done. Purrfect.")).toBeInTheDocument();
    expect(state.history[0]?.outputBytes).toBe(3_000_000);
  });

  it("encodes video as AV1 where the format can hold it", async () => {
    await openVideo();
    const codecs = () => within(screen.getByRole("group", { name: "Video codec" }));
    expect(codecs().getByRole("button", { name: "H.264" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(codecs().getByRole("button", { name: "AV1" }));
    expect(codecs().getByRole("button", { name: "AV1" })).toHaveAttribute("aria-pressed", "true");
    const changes = within(screen.getByRole("list", { name: "Changes" }));
    expect(changes.getByText("AV1")).toBeInTheDocument();

    // WebM can hold AV1 too; MOV can't, so the choice falls back to H.264.
    fireEvent.click(screen.getByRole("button", { name: "WEBM" }));
    expect(codecs().getByRole("button", { name: "AV1" })).toHaveAttribute("aria-pressed", "true");
    expect(codecs().getByRole("button", { name: "VP9" })).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(screen.getByRole("button", { name: "MOV" }));
    expect(codecs().queryByRole("button", { name: "AV1" })).not.toBeInTheDocument();
    expect(codecs().getByRole("button", { name: "H.264" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "MKV" }));
    expect(codecs().getByRole("button", { name: "H.264" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(codecs().getByRole("button", { name: "AV1" }));

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ format: "mkv", videoCodec: "av1" }),
      USUAL,
    );
  });

  it("encodes H.265 into MP4, MOV and MKV, in hardware where it can", async () => {
    vi.mocked(api.ffmpegStatus).mockResolvedValue({ ...status(), hevcHardware: true });
    await openVideo();
    const codecs = () => within(screen.getByRole("group", { name: "Video codec" }));
    // No hardware switch until H.265 is picked.
    expect(screen.queryByRole("switch", { name: "Fast (hardware)" })).not.toBeInTheDocument();
    fireEvent.click(codecs().getByRole("button", { name: "H.265" }));
    expect(codecs().getByRole("button", { name: "H.265" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText(/Plays on Apple devices/)).toBeInTheDocument();
    const changes = () => within(screen.getByRole("list", { name: "Changes" }));
    expect(changes().getByText("H.265")).toBeInTheDocument();

    // MOV keeps it; WebM can't hold it and falls back to VP9.
    fireEvent.click(screen.getByRole("button", { name: "MOV" }));
    expect(codecs().getByRole("button", { name: "H.265" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "WEBM" }));
    expect(codecs().queryByRole("button", { name: "H.265" })).not.toBeInTheDocument();
    expect(codecs().getByRole("button", { name: "VP9" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "MOV" }));
    fireEvent.click(codecs().getByRole("button", { name: "H.265" }));

    const hardware = screen.getByRole("switch", { name: "Fast (hardware)" });
    expect(hardware).toHaveAttribute("aria-checked", "false");
    fireEvent.click(hardware);
    expect(hardware).toHaveAttribute("aria-checked", "true");
    expect(changes().getByText("H.265, hardware")).toBeInTheDocument();
    expect(await screen.findByText(/\(hardware, quality 64\)$/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ format: "mov", videoCodec: "h265", hardware: true }),
      USUAL,
    );
  });

  it("offers no hardware switch, and no H.265, where ffmpeg can't", async () => {
    vi.mocked(api.ffmpegStatus).mockResolvedValue({
      ...status(),
      videoCodecs: ["h264", "vp9", "av1"],
    });
    await openVideo();
    const codecs = within(screen.getByRole("group", { name: "Video codec" }));
    const h265 = codecs.getByRole("button", { name: "H.265" });
    expect(h265).toBeDisabled();
    expect(h265).toHaveAttribute("title", "Your ffmpeg can't write H.265 video");
    expect(screen.queryByRole("switch", { name: "Fast (hardware)" })).not.toBeInTheDocument();
  });

  it("shows the quality in bitrates, updating as the slider moves", async () => {
    await openVideo();
    // The source's bitrates, to compare with.
    expect(screen.getByText(/H264 9 Mbps · AAC 128 kbps/)).toBeInTheDocument();
    const slider = screen.getByRole("slider", { name: "Quality" });
    const numbers = await screen.findByText(/^Audio 128 kbps · Video ~[\d.]+ Mbps \(CRF 24\)$/);
    expect(slider.getAttribute("aria-describedby")).toContain(numbers.id);
    const before = numbers.textContent;

    fireEvent.input(slider, { target: { value: "90" } });
    await waitFor(() =>
      expect(
        screen.getByText(/^Audio 192 kbps · Video ~[\d.]+ Mbps \(CRF 20\)$/),
      ).toBeInTheDocument(),
    );
    expect(numbers.textContent).not.toBe(before);
    const mbps = (text: string | null) => Number(text?.match(/~([\d.]+) Mbps/)?.[1]);
    expect(mbps(numbers.textContent)).toBeGreaterThan(mbps(before));
    expect(api.qualitySummary).toHaveBeenLastCalledWith(
      expect.objectContaining({ path: VIDEO.path }),
      expect.objectContaining({ format: "mp4", quality: 90 }),
    );

    // GIFs have no bitrate to speak of.
    fireEvent.click(screen.getByRole("button", { name: "GIF" }));
    await waitFor(() => expect(document.querySelector(".quality-numbers")).toBeNull());
  });

  it("aims for an exact video bitrate instead of a quality level", async () => {
    await openVideo();
    const modes = () => within(screen.getByRole("group", { name: "Set quality by" }));
    expect(modes().getByRole("button", { name: "Level" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(modes().getByRole("button", { name: "Bitrate" }));

    // It starts at about what the level gave, then takes what is typed.
    const field = screen.getByRole("textbox", { name: "Video bitrate" }) as HTMLInputElement;
    expect(Number(field.value)).toBeGreaterThan(0);
    expect(screen.queryByRole("slider", { name: "Quality" })).not.toBeInTheDocument();
    fireEvent.change(field, { target: { value: "2.5" } });
    expect(await screen.findByText("Audio 128 kbps · Video 2.5 Mbps (2-pass)")).toBeInTheDocument();
    const changes = within(screen.getByRole("list", { name: "Changes" }));
    expect(changes.getByText("2.5 Mbps")).toBeInTheDocument();
    // Out of range is pulled back in.
    fireEvent.change(field, { target: { value: "900" } });
    expect(field.value).toBe("200");
    fireEvent.change(field, { target: { value: "2.5" } });

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ format: "mp4", videoBitrate: 2500 }),
      USUAL,
    );
  });

  it("goes back from an exact bitrate to quality levels", async () => {
    await openVideo();
    const modes = () => within(screen.getByRole("group", { name: "Set quality by" }));
    fireEvent.click(modes().getByRole("button", { name: "Bitrate" }));
    fireEvent.click(modes().getByRole("button", { name: "Level" }));
    expect(screen.getByRole("slider", { name: "Quality" })).toBeInTheDocument();
    // Sound files have no picture to aim for.
    fireEvent.click(screen.getByRole("button", { name: "MP3" }));
    expect(screen.queryByRole("group", { name: "Set quality by" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ format: "mp3", videoBitrate: null }),
      USUAL,
    );
  });

  it("says so when ffmpeg can't read an AV1 file", async () => {
    vi.mocked(api.ffmpegStatus).mockResolvedValue({ ...status(), readsAv1: false });
    vi.mocked(api.probeFile).mockResolvedValue({ ...VIDEO, videoCodec: "av1" });
    await openVideo();
    expect(screen.getByRole("alert")).toHaveTextContent(/AV1, which your ffmpeg can't read/);
    expect(screen.getByRole("button", { name: /Start converting/ })).toBeDisabled();
  });

  it("crops to a shape that follows the picture when it turns", async () => {
    await openVideo();
    expect(screen.queryByRole("group", { name: "Crop shape" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Crop" }));
    expect(screen.getByRole("button", { name: "Crop" })).toHaveAttribute("aria-pressed", "true");
    const shapes = within(screen.getByRole("group", { name: "Crop shape" }));
    expect(shapes.getByRole("button", { name: "Free" })).toHaveAttribute("aria-pressed", "true");
    expect(shapes.getByRole("button", { name: "Reset" })).toBeDisabled();

    fireEvent.click(shapes.getByRole("button", { name: "9:16" }));
    expect(shapes.getByRole("button", { name: "9:16" })).toHaveAttribute("aria-pressed", "true");
    const changes = () =>
      within(screen.getByRole("list", { name: "Changes" }))
        .getAllByRole("listitem")
        .map((li) => li.textContent);
    expect(changes()).toEqual(["Cropped 9:16"]);
    // 9:16 of 1920×1080 is 608×1080, so sizes are offered for a 608 px wide picture.
    expect(screen.getByRole("combobox", { name: "Size" })).toHaveDisplayValue("Original (608p)");

    // Turning the picture turns the crop with it.
    fireEvent.click(screen.getByRole("button", { name: "Rotate 90°" }));
    expect(shapes.getByRole("button", { name: "16:9" })).toHaveAttribute("aria-pressed", "true");
    expect(changes()).toEqual(["Turned 90°", "Cropped 16:9"]);

    // Free keeps the box but lets it change shape.
    fireEvent.click(shapes.getByRole("button", { name: "Free" }));
    expect(changes()).toEqual(["Turned 90°", "Cropped 1080×608"]);
    fireEvent.click(shapes.getByRole("button", { name: "1:1" }));
    fireEvent.click(screen.getByRole("button", { name: "Done cropping" }));
    expect(screen.queryByRole("group", { name: "Crop shape" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ rotate: 90, crop: { aspect: [1, 1], area: null } }),
      USUAL,
    );
  });

  it("resets a crop", async () => {
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: "Crop" }));
    const shapes = within(screen.getByRole("group", { name: "Crop shape" }));
    fireEvent.click(shapes.getByRole("button", { name: "1:1" }));
    fireEvent.click(shapes.getByRole("button", { name: "Reset" }));
    expect(state.draft?.options.crop).toBeNull();
    expect(screen.getByText("Same picture and length, fresh format.")).toBeInTheDocument();
  });

  it("handles progress events that arrive before startJob resolves", async () => {
    vi.mocked(api.startJob).mockImplementation(async (path, options) => {
      applyProgress({
        jobId: 9,
        stage: "running",
        progress: 0.5,
        speed: null,
        outputBytes: null,
        error: null,
      });
      return { id: 9, output: `${path}.${options.format}`, inputs: [VIDEO], command: "" };
    });
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    expect(await screen.findByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
  });

  it("shows failures with a way to retry", async () => {
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    applyProgress({
      jobId: 7,
      stage: "failed",
      progress: 0.1,
      speed: null,
      outputBytes: null,
      error: "Invalid data found",
    });
    expect(await screen.findByText("Invalid data found")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }));
    await waitFor(() => expect(api.startJob).toHaveBeenCalledTimes(2));
  });

  it("walks through several dropped files one after another", async () => {
    render(() => <App />);
    await ffmpegFound();
    openPaths(["/clips/a.mov", "/clips/b.mov"]);
    await screen.findByRole("heading", { name: "a.mov" });
    expect(screen.getByText("Up next")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Convert & next/ }));
    expect(await screen.findByRole("heading", { name: "b.mov" })).toBeInTheDocument();
    expect(screen.getByText("a.mov is in the queue")).toBeInTheDocument();
  });

  it("applies a recipe to picked files", async () => {
    vi.mocked(api.pickFiles).mockResolvedValue([VIDEO.path]);
    render(() => <App />);
    await ffmpegFound();
    fireEvent.click(screen.getByRole("button", { name: /Make a GIF/ }));
    await screen.findByRole("heading", { name: "clip.mov" });
    expect(screen.getByRole("button", { name: "GIF" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("combobox", { name: "Size" })).toHaveValue("480");
    expect(screen.queryByRole("switch", { name: "Keep sound" })).not.toBeInTheDocument();
  });

  it("repacks a JPEG into lossless JPEG XL with libjxl settings", async () => {
    render(() => <App />);
    await ffmpegFound();
    expect(screen.queryByText(/libjxl 0.13.0/)).not.toBeInTheDocument();
    openPaths([PHOTO.path]);
    await screen.findByRole("heading", { name: "cat.jpg" });

    fireEvent.click(screen.getByRole("button", { name: "JXL" }));
    // JPEG input starts lossless: the JPEG is repacked, so quality is moot.
    expect(screen.getByRole("switch", { name: "Lossless" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByText(/Repacks the JPEG without re-encoding/)).toBeInTheDocument();
    expect(screen.queryByRole("slider", { name: "Quality" })).not.toBeInTheDocument();

    // Turning it makes it an ordinary lossless encode.
    fireEvent.click(screen.getByRole("button", { name: "Rotate 90°" }));
    expect(screen.getByText(/every pixel and sample stays put/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("switch", { name: "Lossless" }));
    expect(screen.getByRole("slider", { name: "Quality" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Thorough" }));
    fireEvent.click(screen.getByRole("switch", { name: "Progressive" }));
    expect(await screen.findByText("Saved as cat.jxl next to the original.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      PHOTO.path,
      expect.objectContaining({
        format: "jxl",
        rotate: 90,
        jxl: { lossless: false, effort: 9, progressive: true },
      }),
      USUAL,
    );
  });

  it("turns a video into an animated JPEG XL with auto limits", async () => {
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: "JXL" }));
    expect(screen.getByRole("combobox", { name: "Size" })).toHaveDisplayValue("Auto (720p)");
    expect(screen.queryByRole("switch", { name: "Keep sound" })).not.toBeInTheDocument();
    const changes = within(screen.getByRole("list", { name: "Changes" }));
    expect(changes.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "720p",
      "Animated",
    ]);
  });

  it("changes how a GIF plays and says so, since the preview is a still", async () => {
    await openVideo();
    // Videos keep their own pace for now.
    expect(screen.queryByRole("group", { name: "Speed" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "GIF" }));
    const speed = within(screen.getByRole("group", { name: "Speed" }));
    const direction = within(screen.getByRole("group", { name: "Direction" }));
    expect(speed.getByRole("button", { name: "1×" })).toHaveAttribute("aria-pressed", "true");
    expect(direction.getByRole("button", { name: "Forward" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    const before = screen.getByText(/^About /).textContent;

    fireEvent.click(speed.getByRole("button", { name: "2×" }));
    fireEvent.click(direction.getByRole("button", { name: "Boomerang" }));
    const changes = within(screen.getByRole("list", { name: "Changes" }));
    expect(changes.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "480p",
      "15 fps",
      "2× · Boomerang · 42 s",
    ]);
    // Same length and frame rate, so about the same size.
    expect(screen.getByText(/^About /).textContent).toBe(before);

    fireEvent.click(speed.getByRole("button", { name: "4×" }));
    fireEvent.click(direction.getByRole("button", { name: "Reverse" }));
    expect(changes.getAllByRole("listitem").at(-1)?.textContent).toBe("4× · Reverse · 10.5 s");

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ format: "gif", speed: 4, direction: "reverse" }),
      USUAL,
    );
  });

  it("restores the original JPEG from a repacked JPEG XL", async () => {
    render(() => <App />);
    await ffmpegFound();
    openPaths([PACKED.path]);
    await screen.findByRole("heading", { name: "cat.jxl" });
    expect(screen.getByText(/4032×3024 · JPEG XL/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "JPG" }));
    expect(screen.getByText("Restores the original JPEG, bit for bit.")).toBeInTheDocument();
  });

  it("offers only sound formats and no picture tools for audio", async () => {
    render(() => <App />);
    await ffmpegFound();
    openPaths([SONG.path]);
    await screen.findByRole("heading", { name: "purr.wav" });
    const formats = screen.getByRole("group", { name: "Turn it into" });
    expect(
      within(formats)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["MP3", "M4A", "WAV", "FLAC", "OGG"]);
    expect(screen.getByRole("button", { name: "MP3" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: "Rotate 90°" })).not.toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Trim end" })).toBeInTheDocument();
  });

  it("plays a song, just the kept part or on a loop, with Space as the shortcut", async () => {
    await openSong();
    const play = screen.getByRole("button", { name: "Play" });
    expect(play).toHaveAttribute("aria-keyshortcuts", "Space");
    fireEvent.click(play);
    await settle();
    expect(api.playbackUrl).toHaveBeenCalledWith(SONG.path, false);
    expect(FakeAudio.playing()?.src).toBe("asset://localhost/music/purr.wav");
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
    expect(screen.getByText("/ 1:00.0")).toBeInTheDocument();

    // Space pauses, unless it is pressing a focused button.
    fireEvent.keyDown(screen.getByRole("button", { name: "Pause" }), { key: " " });
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: " " });
    expect(screen.getByRole("button", { name: "Play" })).toBeInTheDocument();
    expect(FakeAudio.playing()).toBeUndefined();

    // Shift+Space plays the selection from where it starts.
    fireEvent.keyDown(screen.getByRole("slider", { name: "Trim start" }), { key: "End" });
    fireEvent.click(screen.getByRole("button", { name: "Loop" }));
    expect(screen.getByRole("button", { name: "Loop" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.keyDown(window, { key: " ", shiftKey: true });
    await settle();
    const selection = screen.getByRole("button", { name: "Play selection" });
    expect(selection).toHaveAttribute("aria-pressed", "true");
    expect(FakeAudio.playing()?.currentTime).toBeCloseTo(41.9, 1);

    // Seeking with the playhead slider.
    fireEvent.click(selection);
    const playhead = screen.getByRole("slider", { name: "Playback position" });
    fireEvent.keyDown(playhead, { key: "Home" });
    fireEvent.keyDown(playhead, { key: "ArrowRight", shiftKey: true });
    expect(playhead).toHaveAttribute("aria-valuenow", "5");
  });

  it("lets you hear even loudness and fades before converting", async () => {
    await openSong();
    expect(screen.queryByText(/as the copy will/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Even out loudness" }));
    expect(screen.getByText("You hear the even loudness as the copy will.")).toBeInTheDocument();
    fireEvent.click(
      within(screen.getByRole("group", { name: "Fade in" })).getByRole("button", { name: "1 s" }),
    );
    expect(
      screen.getByText("You hear the fades and even loudness as the copy will."),
    ).toBeInTheDocument();

    // Loudness is measured once, when first heard.
    expect(api.loudness).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await settle();
    expect(api.loudness).toHaveBeenCalledTimes(1);
    expect(api.loudness).toHaveBeenCalledWith(SONG.path);
  });

  it("says when a file can't be played and stops playing on leaving the editor", async () => {
    vi.mocked(api.playbackUrl).mockRejectedValueOnce("ffmpeg was not found");
    await openSong();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("ffmpeg was not found");

    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await settle();
    expect(FakeAudio.playing()).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(FakeAudio.playing()).toBeUndefined();
  });

  it("plays the sound of a video, with the picture following when paused", async () => {
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await settle();
    // The preview opened at 1 s, so the sound starts there.
    const audio = FakeAudio.playing() as FakeAudio;
    expect(audio.src).toBe("asset://localhost/clips/clip.mov");
    expect(audio.currentTime).toBe(1);
    audio.currentTime = 7;
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(screen.getByText("0:07.0", { selector: ".frame-time" })).toBeInTheDocument();
  });

  it("plays finished sound files from the Queue and History", async () => {
    await openSong();
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    applyProgress({
      jobId: 7,
      stage: "done",
      progress: 1,
      speed: null,
      outputBytes: 9,
      error: null,
    });

    fireEvent.click(await screen.findByRole("button", { name: "Play purr.mp3" }));
    await settle();
    expect(api.playbackUrl).toHaveBeenCalledWith("/music/purr.mp3", false);
    expect(screen.getByRole("button", { name: "Pause purr.mp3" })).toBeInTheDocument();
    expect(screen.getByText("0:00 / 1:00")).toBeInTheDocument();

    // Leaving the screen stops it; History has its own button.
    fireEvent.click(screen.getByRole("button", { name: "History" }));
    expect(FakeAudio.playing()).toBeUndefined();
    fireEvent.click(await screen.findByRole("button", { name: "Play purr.mp3" }));
    await settle();
    expect(FakeAudio.playing()?.src).toBe("asset://localhost/music/purr.mp3");
  });

  it("checks for an update, installs it and restarts", async () => {
    vi.mocked(api.checkForUpdate).mockResolvedValue({
      version: "0.2.0",
      currentVersion: "0.1.2",
      date: null,
      notes: "Purrs faster.",
    });
    let finish = () => {};
    vi.mocked(api.installUpdate).mockImplementation(
      (onProgress) =>
        new Promise((resolve) => {
          onProgress({ downloaded: 512 * 1024, total: 2 * 1024 * 1024 });
          finish = resolve;
        }),
    );
    render(() => <App />);
    navigate("settings");
    fireEvent.click(await screen.findByRole("button", { name: /Check for updates/ }));
    expect(await screen.findByText("CatConvert 0.2.0 is out")).toBeInTheDocument();
    expect(screen.getByText("Purrs faster.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Download and install/ }));
    expect(
      await screen.findByRole("progressbar", { name: "Downloading the update" }),
    ).toHaveAttribute("aria-valuenow", "25");
    finish();
    expect(await screen.findByText("CatConvert 0.2.0 is installed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Restart now/ }));
    expect(api.relaunch).toHaveBeenCalledOnce();
  });

  it("downloads AI upscaling, tests it and removes it again", async () => {
    const missing: UpscalerInfo = {
      dir: "/data/upscaler",
      binary: null,
      origin: null,
      models: [],
      missing: ["upscaler", "models", "generalModel"],
      outdated: false,
      canDownload: true,
      downloadBytes: 51_816_585,
      gpu: null,
      busy: false,
      unpinned: false,
    };
    const ready: UpscalerInfo = {
      ...missing,
      binary: "/data/upscaler/catconvert-upscaler",
      origin: "installed",
      models: ["general", "photo", "anime", "anime-fast"],
      missing: [],
      downloadBytes: 0,
      gpu: { status: "ready", gpu: "Apple M5" },
    };
    vi.mocked(api.upscalerStatus).mockResolvedValue(missing);
    let finish = (_info: UpscalerInfo) => {};
    let progress = (_progress: UpscalerDownload) => {};
    vi.mocked(api.downloadUpscaler).mockImplementation(
      (onProgress) =>
        new Promise((resolve) => {
          onProgress({ stage: "downloading", received: 25_000_000, total: 50_000_000 });
          progress = onProgress;
          finish = resolve;
        }),
    );
    render(() => <App />);
    navigate("settings");
    // Before installing: what comes with it, and which one to start with.
    const guide = await screen.findByRole("list", { name: "What you get" });
    expect(
      within(guide)
        .getAllByRole("listitem")
        .map((li) => li.firstElementChild?.textContent),
    ).toEqual(["PhotosRecommended", "Photos, most detail", "Drawings and anime", "Anime, quick"]);
    fireEvent.click(
      await screen.findByRole("button", { name: "Download and install (about 51.8 MB)" }),
    );
    expect(
      await screen.findByRole("progressbar", { name: "Downloading AI upscaling" }),
    ).toHaveAttribute("aria-valuenow", "50");
    expect(screen.getByText("25.0 MB of 50.0 MB")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    // Once the files are in, it says what it is busy with, and can't be cancelled.
    progress({ stage: "installing", received: 50_000_000, total: 50_000_000 });
    expect(await screen.findByText("Installing AI upscaling…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
    progress({ stage: "testing", received: 50_000_000, total: 50_000_000 });
    expect(await screen.findByText("Trying it on your graphics card…")).toBeInTheDocument();
    vi.mocked(api.ffmpegStatus).mockResolvedValue({ ...status(), aiModels: ready.models });
    finish(ready);
    expect(await screen.findByText("Installed")).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Which one to pick" })).toBeInTheDocument();
    expect(screen.getByText("Works on Apple M5.")).toBeInTheDocument();
    expect(await screen.findByText("AI upscaling is ready on Apple M5")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Download/ })).not.toBeInTheDocument();
    await waitFor(() => expect(state.ffmpeg?.aiModels).toEqual(ready.models));

    vi.mocked(api.checkUpscalerGpu).mockResolvedValue({
      ...ready,
      gpu: { status: "blackOutput", gpu: "Old GPU" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Test again/ }));
    expect(
      await screen.findByText(
        "The graphics driver makes black pictures. Update it, then test again.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Installed, but it can't run here")).toBeInTheDocument();
    // A driver that makes black pictures takes the models off the menu.
    expect(aiModels()).toEqual([]);

    vi.mocked(api.removeUpscaler).mockResolvedValue(missing);
    vi.mocked(api.ffmpegStatus).mockResolvedValue({ ...status(), aiModels: [] });
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(await screen.findByText("Not installed")).toBeInTheDocument();
    expect(api.removeUpscaler).toHaveBeenCalledOnce();
  });

  it("installs AI upscaling from files where it can't download", async () => {
    vi.mocked(api.upscalerStatus).mockResolvedValue({
      dir: "/data/upscaler",
      binary: null,
      origin: null,
      models: [],
      missing: ["upscaler", "models", "generalModel"],
      outdated: false,
      canDownload: false,
      downloadBytes: 51_816_585,
      gpu: null,
      busy: false,
      unpinned: false,
    });
    vi.mocked(api.pickUpscalerFiles).mockResolvedValue(["/dl/catconvert-upscaler.zip"]);
    vi.mocked(api.installUpscalerFiles).mockResolvedValue({
      dir: "/data/upscaler",
      binary: "/data/upscaler/catconvert-upscaler",
      origin: "installed",
      models: ["photo", "anime", "anime-fast"],
      missing: ["generalModel"],
      outdated: false,
      canDownload: false,
      downloadBytes: 4_885_111,
      gpu: { status: "noGpu" },
      busy: false,
      unpinned: false,
    });
    render(() => <App />);
    navigate("settings");
    expect(await screen.findByText("Not installed")).toBeInTheDocument();
    // Where no upscaler is built for the computer, the models still download.
    expect(screen.queryByRole("button", { name: /Download and install/ })).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Download the models (about 51.8 MB)" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/no upscaler to download for this kind of computer/),
    ).toBeInTheDocument();
    // The files to pick are spelled out, since they are the only way to the upscaler.
    expect(screen.getByRole("button", { name: "Install from files instead" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByText(/realesr-general-x4v3\.pth/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Install from file…" }));
    expect(await screen.findByText("Installed, but it can't run here")).toBeInTheDocument();
    expect(api.installUpscalerFiles).toHaveBeenCalledWith(["/dl/catconvert-upscaler.zip"]);
    expect(screen.getByText(/no graphics card AI upscaling can use/)).toBeInTheDocument();
    expect(screen.getByText("Still missing: the Photos model.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Download the rest (about 4.9 MB)" })).toBeEnabled();
    // What isn't there yet is marked, and the suggestion moves to what is.
    expect(
      within(screen.getByRole("list", { name: "Which one to pick" }))
        .getAllByRole("listitem")
        .map((li) => li.firstElementChild?.textContent),
    ).toEqual([
      "PhotosNot installed",
      "Photos, most detailRecommended",
      "Drawings and anime",
      "Anime, quick",
    ]);
    expect(
      await screen.findByText("Installed. A file is still missing: see Settings → AI upscaling"),
    ).toBeInTheDocument();
  });

  it("keeps a failed AI upscaling download on the page until the next try", async () => {
    const missing: UpscalerInfo = {
      dir: "/data/upscaler",
      binary: null,
      origin: null,
      models: [],
      missing: ["upscaler", "models", "generalModel"],
      outdated: false,
      canDownload: true,
      downloadBytes: 51_816_585,
      gpu: null,
      busy: false,
      unpinned: false,
    };
    vi.mocked(api.upscalerStatus).mockResolvedValue(missing);
    vi.mocked(api.downloadUpscaler).mockRejectedValueOnce(
      "couldn't download realesr-general-x4v3.pth: the server didn't answer. Check your internet connection, then try again",
    );
    render(() => <App />);
    navigate("settings");
    fireEvent.click(await screen.findByRole("button", { name: /Download and install/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't download realesr-general-x4v3.pth: the server didn't answer.",
    );
    expect(screen.getByText("Not installed")).toBeInTheDocument();

    // Stopping a download is no error.
    let stop = (_reason: string) => {};
    vi.mocked(api.downloadUpscaler).mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          stop = reject;
        }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Download and install/ }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(api.cancelUpscalerDownload).toHaveBeenCalledOnce();
    stop("the download was cancelled");
    expect(await screen.findByText("Download cancelled")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /Download and install/ })).toBeEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("downloads the upscaler in a build from source, saying how it is checked", async () => {
    const missing: UpscalerInfo = {
      dir: "/data/upscaler",
      binary: null,
      origin: null,
      models: [],
      missing: ["upscaler", "models", "generalModel"],
      outdated: false,
      canDownload: false,
      downloadBytes: 51_816_585,
      gpu: null,
      busy: false,
      unpinned: true,
    };
    const models: UpscalerInfo = {
      ...missing,
      models: [],
      missing: ["upscaler"],
      downloadBytes: 0,
    };
    vi.mocked(api.upscalerStatus).mockResolvedValue(missing);
    // The release has no upscaler: the models are installed all the same.
    vi.mocked(api.downloadUpscaler).mockImplementation(async () => {
      vi.mocked(api.upscalerStatus).mockResolvedValue(models);
      throw "the release of CatConvert 0.1.6 has no upscaler to download";
    });
    render(() => <App />);
    navigate("settings");
    expect(await screen.findByText(/built from source/)).toBeInTheDocument();
    expect(screen.getByText(/checksum published with the release/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install from files instead" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    fireEvent.click(screen.getByRole("button", { name: "Download and install (about 51.8 MB)" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The release of CatConvert 0.1.6 has no upscaler to download",
    );
    expect(await screen.findByText("Partly installed")).toBeInTheDocument();
    expect(screen.getByText("Still missing: the upscaler.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Download the rest" })).toBeEnabled();
  });

  it("says which file was wrong when installing from files", async () => {
    vi.mocked(api.upscalerStatus).mockResolvedValue({
      dir: "/data/upscaler",
      binary: null,
      origin: null,
      models: [],
      missing: ["upscaler", "models", "generalModel"],
      outdated: false,
      canDownload: true,
      downloadBytes: 51_816_585,
      gpu: null,
      busy: false,
      unpinned: false,
    });
    vi.mocked(api.pickUpscalerFiles).mockResolvedValue(["/dl/holiday.jpg"]);
    vi.mocked(api.installUpscalerFiles).mockRejectedValue(
      "holiday.jpg isn't one of the files AI upscaling installs from",
    );
    render(() => <App />);
    navigate("settings");
    // Where the app can download, the files are a way round it, kept out of the way.
    const instead = await screen.findByRole("button", { name: "Install from files instead" });
    expect(instead).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("button", { name: "Install from file…" })).not.toBeInTheDocument();
    fireEvent.click(instead);
    fireEvent.click(screen.getByRole("button", { name: "Install from file…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Holiday.jpg isn't one of the files AI upscaling installs from",
    );
  });

  it("says when CatConvert is up to date, and why a check failed", async () => {
    vi.mocked(api.checkForUpdate)
      .mockResolvedValueOnce(null)
      .mockRejectedValueOnce("could not fetch a valid release JSON");
    render(() => <App />);
    navigate("settings");
    fireEvent.click(await screen.findByRole("button", { name: /Check for updates/ }));
    expect(await screen.findByText("You're up to date.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Check for updates/ }));
    expect(await screen.findByText("Couldn't check for updates")).toBeInTheDocument();
    expect(screen.getByText("Could not fetch a valid release JSON")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Try again/ })).toBeEnabled();
  });

  it("estimates the output size as the quality changes", async () => {
    render(() => <App />);
    await ffmpegFound();
    openPaths([VIDEO.path]);
    await screen.findByRole("heading", { name: "clip.mov" });

    const slider = screen.getByRole("slider", { name: "Quality" });
    expect(slider).toHaveAccessibleDescription(/^About [\d.]+ MB · \d+% smaller$/);
    const id = slider.getAttribute("aria-describedby") ?? "";
    const text = () => document.getElementById(id)?.textContent;
    const atDefault = text();
    fireEvent.input(slider, { target: { value: "0" } });
    expect(text()).not.toBe(atDefault);
  });

  it("starts files from the defaults chosen in Settings and remembers them", async () => {
    render(() => <App />);
    await ffmpegFound();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.change(screen.getByRole("combobox", { name: "Videos become" }), {
      target: { value: "mkv" },
    });
    fireEvent.input(screen.getByRole("slider", { name: "Quality" }), { target: { value: "85" } });
    expect(screen.getByRole("button", { name: "Keep all but location" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove everything" }));
    expect(JSON.parse(localStorage.getItem("catconvert.settings.v1") ?? "{}")).toEqual({
      ...DEFAULTS,
      formats: { video: "mkv", audio: null, image: null },
      quality: 85,
      metadata: "none",
    });

    openPaths([VIDEO.path]);
    await screen.findByRole("heading", { name: "clip.mov" });
    expect(screen.getByRole("button", { name: "MKV" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("slider", { name: "Quality" })).toHaveValue("85");
    expect(screen.getByRole("button", { name: "Remove everything" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("reads the metadata switch of earlier versions", async () => {
    localStorage.setItem(
      "catconvert.settings.v1",
      JSON.stringify({ formats: {}, quality: 60, keepMetadata: true }),
    );
    const recipe = {
      id: "custom-1",
      title: "Old",
      options: { format: "mp4", keepMetadata: false },
    };
    localStorage.setItem("catconvert.recipes.v1", JSON.stringify([recipe]));
    resetStore();
    expect(state.settings).toEqual({
      ...DEFAULTS,
      formats: { video: null, audio: null, image: null },
      quality: 60,
      metadata: "keep",
    });
    expect(state.recipes[0]?.options).toEqual({ format: "mp4", metadata: "none" });
    await openVideo();
    expect(screen.getByRole("button", { name: "Keep everything" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("tells when a photo knows where it was taken and drops it, repack and all", async () => {
    vi.mocked(api.probeFile).mockResolvedValue({ ...PHOTO, hasLocation: true });
    render(() => <App />);
    await ffmpegFound();
    openPaths([PHOTO.path]);
    await screen.findByRole("heading", { name: "cat.jpg" });
    expect(screen.getByText(/This photo knows where it was taken/)).toHaveTextContent(
      "This photo knows where it was taken. The copy won't.",
    );
    const changes = () =>
      within(screen.getByRole("list", { name: "Changes" }))
        .getAllByRole("listitem")
        .map((li) => li.textContent);
    expect(changes()).toContain("No location");

    // The lossless JPEG XL repack stays available without the location.
    fireEvent.click(screen.getByRole("button", { name: "JXL" }));
    expect(screen.getByText(/Repacks the JPEG/)).toHaveTextContent(
      "Repacks the JPEG without re-encoding: smaller, and the original can be restored exactly, minus its location.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove everything" }));
    expect(screen.getByText(/Repacks the JPEG/)).toHaveTextContent(/minus its metadata\.$/);
    fireEvent.click(screen.getByRole("button", { name: "Keep everything" }));
    expect(screen.getByText(/The copy will too\./)).toBeInTheDocument();
    expect(screen.queryByText(/minus its/)).not.toBeInTheDocument();

    // ffmpeg writes no EXIF into JPG, so there is nothing to choose.
    fireEvent.click(screen.getByRole("button", { name: "JPG" }));
    expect(screen.queryByRole("button", { name: "Keep everything" })).not.toBeInTheDocument();
    expect(
      screen.getByText("JPG files come out without dates, place or camera details."),
    ).toBeInTheDocument();
    expect(screen.getByText(/The copy won't\./)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "JXL" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep all but location" }));
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      PHOTO.path,
      expect.objectContaining({ format: "jxl", metadata: "noLocation" }),
      USUAL,
    );
  });

  it("saves the editor's settings as a recipe and reuses it", async () => {
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: "WEBM" }));
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "720" } });
    fireEvent.click(screen.getByRole("button", { name: /Save these settings as a recipe/ }));
    const name = screen.getByRole("textbox", { name: "Recipe name" });
    expect(name).toHaveValue("My WEBM");
    fireEvent.input(name, { target: { value: "Small webm" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByText("Saved “Small webm” to your recipes")).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem("catconvert.recipes.v1") ?? "[]")).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(await screen.findByRole("button", { name: "Recipes" }));
    vi.mocked(api.pickFiles).mockResolvedValue(["/clips/other.mov"]);
    fireEvent.click(screen.getByRole("button", { name: /^Small webm/ }));
    await screen.findByRole("heading", { name: "other.mov" });
    expect(screen.getByRole("button", { name: "WEBM" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("combobox", { name: "Size" })).toHaveValue("720");

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(await screen.findByRole("button", { name: "Recipes" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove Small webm" }));
    expect(screen.queryByRole("button", { name: /^Small webm/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(screen.getByRole("button", { name: /^Small webm/ })).toBeInTheDocument();
  });

  it("leads back to the open file after a detour through Settings", async () => {
    await openVideo();
    navigate("settings");
    await screen.findByRole("heading", { name: "Settings" });

    // Files dropped meanwhile wait behind the open one, with a way back to it.
    openPaths(["/clips/b.mov"]);
    expect(screen.getByText("1 more waiting their turn")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open editor" }));
    expect(screen.getByRole("heading", { name: "clip.mov" })).toBeInTheDocument();

    // The Convert tab shows the editor rather than an empty drop zone.
    navigate("settings");
    fireEvent.click(screen.getByRole("button", { name: "Convert" }));
    expect(screen.getByRole("heading", { name: "clip.mov" })).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Drop files on the kitty" }),
    ).not.toBeInTheDocument();
  });

  it("converts every waiting file with one set of settings", async () => {
    render(() => <App />);
    await ffmpegFound();
    openPaths(["/clips/a.mov", "/clips/b.mov", SONG.path]);
    await screen.findByRole("heading", { name: "a.mov" });
    fireEvent.click(screen.getByRole("button", { name: "WEBM" }));
    fireEvent.keyDown(screen.getByRole("slider", { name: "Trim start" }), {
      key: "ArrowRight",
      shiftKey: true,
    });
    fireEvent.click(screen.getByRole("button", { name: /Convert all 3 like this/ }));

    // The song can't become a WEBM, so it comes back to the editor on its own.
    await screen.findByRole("heading", { name: "purr.wav" });
    expect(screen.getByText("purr.wav needs its own settings")).toBeInTheDocument();
    expect(api.startJob).toHaveBeenCalledTimes(2);
    expect(api.startJob).toHaveBeenNthCalledWith(
      1,
      "/clips/a.mov",
      expect.objectContaining({ format: "webm", trimStart: 1 }),
      USUAL,
    );
    expect(api.startJob).toHaveBeenNthCalledWith(
      2,
      "/clips/b.mov",
      expect.objectContaining({ format: "webm", trimStart: null }),
      USUAL,
    );
  });

  describe("where it's saved", () => {
    const nameField = () => screen.getByRole("textbox", { name: "File name" });

    it("saves one file in a chosen folder under a chosen name", async () => {
      vi.mocked(api.chooseSavePath).mockResolvedValue("/exports/Holiday.mov");
      await openVideo();
      expect(
        await screen.findByText("Saved as clip.mp4 next to the original."),
      ).toBeInTheDocument();

      // Change… starts the save dialog at the suggested path.
      fireEvent.click(screen.getByRole("button", { name: "Change…" }));
      await waitFor(() => expect(nameField()).toHaveValue("Holiday"));
      expect(api.chooseSavePath).toHaveBeenCalledWith("/clips/clip.mp4", "mp4", "MP4");
      expect(await screen.findByText(/in exports/)).toBeInTheDocument();

      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      // The system's save dialog already asked before replacing anything.
      await waitFor(() =>
        expect(api.startJob).toHaveBeenCalledWith(VIDEO.path, expect.anything(), {
          folder: "/exports",
          name: "Holiday",
          collision: "overwrite",
        }),
      );
      expect(api.askReplace).not.toHaveBeenCalled();
    });

    it("asks before replacing a file with the typed name", async () => {
      vi.mocked(api.outputPreview).mockImplementation(async (_info, _options, target) =>
        target.name === "taken"
          ? {
              path: `/clips/taken${target.collision === "overwrite" ? "" : " 2"}.mp4`,
              exists: true,
            }
          : { path: "/clips/clip.mp4", exists: false },
      );
      await openVideo();
      fireEvent.input(nameField(), { target: { value: "taken" } });
      expect(
        await screen.findByText(/Saved as taken 2\.mp4 next to the original\. The name is taken/),
      ).toBeInTheDocument();

      // Cancel converts nothing.
      vi.mocked(api.askReplace).mockResolvedValueOnce("cancel");
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await waitFor(() => expect(api.askReplace).toHaveBeenCalledWith("taken.mp4", "/clips"));
      expect(api.startJob).not.toHaveBeenCalled();
      expect(screen.getByRole("heading", { name: "clip.mov" })).toBeInTheDocument();

      // Keep both numbers the new file.
      vi.mocked(api.askReplace).mockResolvedValueOnce("keepBoth");
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await waitFor(() =>
        expect(api.startJob).toHaveBeenCalledWith(VIDEO.path, expect.anything(), {
          folder: null,
          name: "taken",
          collision: "keepBoth",
        }),
      );
    });

    it("explains names that won't work", async () => {
      vi.mocked(api.outputPreview).mockImplementation(async (_info, _options, target) => {
        if (target.name?.includes(":"))
          throw "file names can't contain : (nor any of < > : \" / \\ | ? *)";
        return { path: `/clips/${target.name ?? "clip"}.mp4`, exists: false };
      });
      await openVideo();
      const start = screen.getByRole("button", { name: /Start converting/ });

      fireEvent.input(nameField(), { target: { value: "holiday.mov" } });
      expect(
        screen.getByText("The format decides the extension, so it's saved as .mp4, not .mov."),
      ).toBeInTheDocument();

      fireEvent.input(nameField(), { target: { value: "a:b" } });
      expect(await screen.findByRole("alert")).toHaveTextContent("file names can't contain :");
      expect(nameField()).toHaveAttribute("aria-invalid", "true");
      expect(start).toBeDisabled();

      fireEvent.input(nameField(), { target: { value: "ab" } });
      await waitFor(() => expect(start).toBeEnabled());
    });

    it("names a batch with a pattern in one folder, never replacing", async () => {
      vi.mocked(api.pickFolder).mockResolvedValue("/exports");
      render(() => <App />);
      await ffmpegFound();
      openPaths(["/clips/a.mov", "/clips/b.mov"]);
      await screen.findByRole("heading", { name: "a.mov" });

      const pattern = screen.getByRole("textbox", { name: "File name pattern" });
      expect(pattern).toHaveAttribute("placeholder", "{name}");
      fireEvent.input(pattern, { target: { value: "{name}-" } });
      fireEvent.click(screen.getByRole("button", { name: "{format}" }));
      expect(pattern).toHaveValue("{name}-{format}");

      expect(
        await screen.findByText("Saved next to the original; this one as a.mp4."),
      ).toBeInTheDocument();

      // Only the folder is picked: each file gets its own name.
      fireEvent.click(screen.getByRole("button", { name: "Change…" }));
      await waitFor(() => expect(state.draft?.output.folder).toBe("/exports"));
      expect(api.pickFolder).toHaveBeenCalledWith("/clips");
      expect(api.chooseSavePath).not.toHaveBeenCalled();

      fireEvent.click(screen.getByRole("button", { name: /Convert all 2 like this/ }));
      await waitFor(() => expect(api.startJob).toHaveBeenCalledTimes(2));
      const target = { folder: "/exports", name: "{name}-{format}", collision: "keepBoth" };
      expect(api.startJob).toHaveBeenNthCalledWith(1, "/clips/a.mov", expect.anything(), target);
      expect(api.startJob).toHaveBeenNthCalledWith(2, "/clips/b.mov", expect.anything(), target);
    });

    it("works with the Save menu: a picked folder, the output folder, replacing", async () => {
      vi.mocked(api.chooseSavePath).mockResolvedValue("/exports/Holiday.mp4");
      updateSettings({ replaceConfirmed: true });
      await openVideo();
      const save = screen.getByRole("combobox", { name: "Save" });

      // Replacing keeps the original's name: no name to type.
      fireEvent.change(save, { target: { value: "replaceInput" } });
      await waitFor(() => expect(save).toHaveDisplayValue("In place of the original"));
      expect(screen.queryByRole("textbox", { name: "File name" })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Change…" })).not.toBeInTheDocument();
      fireEvent.change(save, { target: { value: "nextToInput" } });
      await waitFor(() => expect(save).toHaveDisplayValue("Next to the original"));

      // A folder picked with Change… shows in the menu, and saves there.
      fireEvent.click(screen.getByRole("button", { name: "Change…" }));
      await waitFor(() => expect(save).toHaveDisplayValue("In exports"));
      expect(screen.getByRole("textbox", { name: "File name" })).toHaveValue("Holiday");

      // Choosing from the menu again drops the picked folder but keeps the name.
      fireEvent.change(save, { target: { value: "nextToInput" } });
      await waitFor(() => expect(save).toHaveDisplayValue("Next to the original"));
      expect(state.draft?.output).toEqual({ folder: null, name: "Holiday", collision: null });
      expect(state.lastFolder).toBeNull();

      // Replacing drops a chosen name too.
      fireEvent.change(save, { target: { value: "replaceInput" } });
      await waitFor(() => expect(save).toHaveDisplayValue("In place of the original"));
      expect(state.draft?.output).toEqual({ folder: null, name: null, collision: null });
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await waitFor(() =>
        expect(api.startJob).toHaveBeenCalledWith(
          VIDEO.path,
          expect.objectContaining({ destination: { kind: "replaceInput" } }),
          USUAL,
        ),
      );
    });

    it("keeps the chosen folder for frames and the next files", async () => {
      vi.mocked(api.chooseSavePath).mockResolvedValue("/exports/clip.mp4");
      vi.mocked(api.saveFrame).mockResolvedValue({
        output: "/exports/clip-frame-0m01s.png",
        outputBytes: 1,
      });
      render(() => <App />);
      await ffmpegFound();
      openPaths([VIDEO.path]);
      await screen.findByRole("heading", { name: "clip.mov" });
      fireEvent.click(screen.getByRole("button", { name: "Change…" }));
      await waitFor(() => expect(state.draft?.output.folder).toBe("/exports"));

      // Frames go to the folder under their own name.
      fireEvent.click(screen.getByRole("button", { name: "Save this frame" }));
      await screen.findByText("Saved frame");
      expect(vi.mocked(api.saveFrame).mock.calls[0]?.[3]).toEqual({
        folder: "/exports",
        name: null,
        collision: "keepBoth",
      });

      // The next file starts in the same folder, under its usual name.
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await waitFor(() => expect(api.startJob).toHaveBeenCalledTimes(1));
      openPaths(["/clips/next.mov"]);
      await screen.findByRole("heading", { name: "next.mov" });
      expect(state.draft?.output).toEqual({ folder: "/exports", name: null, collision: null });

      // Reset goes back next to the original, for later files too.
      fireEvent.click(screen.getByRole("button", { name: "Reset" }));
      expect(state.draft?.output).toEqual({ folder: null, name: null, collision: null });
      expect(state.lastFolder).toBeNull();
    });
  });

  describe("editing the command", () => {
    const GENERATED = "ffmpeg -i /clips/clip.mov -c:v libx264 -crf 24 /clips/clip.mp4";

    beforeEach(() => {
      vi.mocked(api.previewCommand).mockResolvedValue(GENERATED);
      vi.mocked(api.checkCommand).mockImplementation(async (_info, _options, text) => {
        if (!text.startsWith("ffmpeg ")) {
          throw `${text.split(" ")[0]} isn't a tool CatConvert runs. Commands can use ffmpeg, cjxl, djxl and catconvert scrub.`;
        }
        if (text.includes(VIDEO.path.replace(".mov", ".mp4")) === false) {
          throw "This command would write over the original, clip.mov. CatConvert never does that.";
        }
        return text;
      });
      vi.mocked(api.startCustomJob).mockImplementation(async (path, _options, text) => ({
        id: 11,
        output: path.replace(/\.\w+$/, ".mp4"),
        inputs: [VIDEO],
        command: text,
      }));
    });

    async function editCommand() {
      await openVideo();
      fireEvent.click(screen.getByText("Show command"));
      await screen.findByText(GENERATED);
      fireEvent.click(screen.getByRole("button", { name: "Edit" }));
      const field = screen.getByRole("textbox", { name: "Command" });
      expect(field).toHaveValue(GENERATED);
      return field;
    }

    it("checks an edited command as it's typed and queues it as written", async () => {
      const field = await editCommand();
      // The settings stand aside while the edited command applies.
      expect(screen.getByText(/these settings no longer apply/)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "WEBM" })).toBeDisabled();
      expect(screen.queryByRole("button", { name: "Rotate 90°" })).not.toBeInTheDocument();
      // The trim is the command's now, but the sound can still be played.
      fireEvent.keyDown(screen.getByRole("slider", { name: "Trim start" }), {
        key: "ArrowRight",
        shiftKey: true,
      });
      expect(state.draft?.options.trimStart).toBeNull();
      expect(screen.getByRole("button", { name: "Play" })).toBeEnabled();
      const changes = within(screen.getByRole("list", { name: "Changes" }));
      expect(changes.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
        "Custom command",
      ]);

      fireEvent.input(field, { target: { value: "rm -rf /" } });
      expect(await screen.findByRole("alert")).toHaveTextContent("rm isn't a tool CatConvert runs");
      expect(field).toHaveAttribute("aria-invalid", "true");
      expect(screen.getByRole("button", { name: /Start converting/ })).toBeDisabled();

      fireEvent.input(field, {
        target: { value: GENERATED.replace("/clips/clip.mp4", VIDEO.path) },
      });
      expect(await screen.findByText(/would write over the original/)).toBeInTheDocument();

      const edited = GENERATED.replace("-crf 24", "-crf 18");
      fireEvent.input(field, { target: { value: edited } });
      await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
      const start = screen.getByRole("button", { name: /Start converting/ });
      expect(start).toBeEnabled();
      fireEvent.click(start);
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startCustomJob).toHaveBeenCalledWith(
        VIDEO.path,
        expect.objectContaining({ format: "mp4" }),
        edited,
        USUAL,
      );
      expect(api.startJob).not.toHaveBeenCalled();

      // Progress shows in the queue as for any job, and a retry runs the same command.
      applyProgress({
        jobId: 11,
        stage: "running",
        progress: 0.3,
        speed: null,
        outputBytes: null,
        error: null,
      });
      expect(await screen.findByRole("progressbar")).toHaveAttribute("aria-valuenow", "30");
      applyProgress({
        jobId: 11,
        stage: "failed",
        progress: 0.3,
        speed: null,
        outputBytes: null,
        error: "Unknown encoder",
      });
      fireEvent.click(await screen.findByRole("button", { name: /Retry/ }));
      await waitFor(() => expect(api.startCustomJob).toHaveBeenCalledTimes(2));
      expect(api.startCustomJob).toHaveBeenLastCalledWith(
        VIDEO.path,
        expect.anything(),
        edited,
        USUAL,
      );
    });

    it("goes back to the generated command", async () => {
      const field = await editCommand();
      fireEvent.input(field, { target: { value: `${GENERATED} -tune film` } });
      fireEvent.click(screen.getAllByRole("button", { name: /Reset to generated/ })[0] as Element);
      expect(screen.queryByRole("textbox", { name: "Command" })).not.toBeInTheDocument();
      expect(screen.getByText(GENERATED)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "WEBM" })).toBeEnabled();
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startJob).toHaveBeenCalledTimes(1);
      expect(api.startCustomJob).not.toHaveBeenCalled();
    });

    it("saves an edited command in a recipe and fills it in for other files", async () => {
      const template = "ffmpeg -i '{input}' -c:v libx264 -crf 18 '{output}'";
      vi.mocked(api.commandTemplate).mockResolvedValue(template);
      vi.mocked(api.fillCommand).mockImplementation(async (info, _options, saved) =>
        saved.replace("'{input}'", info.path).replace("'{output}'", "/clips/other.mp4"),
      );
      const field = await editCommand();
      const edited = GENERATED.replace("-crf 24", "-crf 18");
      fireEvent.input(field, { target: { value: edited } });
      fireEvent.click(screen.getByRole("button", { name: /Save these settings as a recipe/ }));
      expect(screen.getByText(/Keeps the edited command as written/)).toBeInTheDocument();
      fireEvent.input(screen.getByRole("textbox", { name: "Recipe name" }), {
        target: { value: "Crisp" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      await screen.findByText("Saved “Crisp” to your recipes");
      expect(api.commandTemplate).toHaveBeenCalledWith(VIDEO.path, edited);
      const [saved] = JSON.parse(localStorage.getItem("catconvert.recipes.v1") ?? "[]");
      expect(saved).toMatchObject({ title: "Crisp", detail: "Custom command", command: template });

      fireEvent.click(screen.getByRole("button", { name: "Back" }));
      fireEvent.click(await screen.findByRole("button", { name: "Recipes" }));
      vi.mocked(api.pickFiles).mockResolvedValue(["/clips/other.mov"]);
      fireEvent.click(screen.getByRole("button", { name: /^Crisp/ }));
      await screen.findByRole("heading", { name: "other.mov" });
      expect(await screen.findByRole("textbox", { name: "Command" })).toHaveValue(
        "ffmpeg -i /clips/other.mov -c:v libx264 -crf 18 /clips/other.mp4",
      );
    });

    it("runs an edited command for every waiting file, with each file's paths", async () => {
      const template = "ffmpeg -i '{input}' -crf 18 '{output}'";
      vi.mocked(api.commandTemplate).mockResolvedValue(template);
      render(() => <App />);
      await ffmpegFound();
      openPaths(["/clips/a.mov", "/clips/b.mov", SONG.path]);
      await screen.findByRole("heading", { name: "a.mov" });
      fireEvent.click(screen.getByText("Show command"));
      await screen.findByText(GENERATED);
      fireEvent.click(screen.getByRole("button", { name: "Edit" }));
      const edited = GENERATED.replace("-crf 24", "-crf 18");
      fireEvent.input(screen.getByRole("textbox", { name: "Command" }), {
        target: { value: edited },
      });
      fireEvent.click(screen.getByRole("button", { name: /Convert all 3 like this/ }));
      await screen.findByRole("heading", { name: "purr.wav" });
      expect(api.startCustomJob).toHaveBeenCalledTimes(2);
      expect(api.startCustomJob).toHaveBeenNthCalledWith(
        1,
        "/clips/a.mov",
        expect.anything(),
        edited,
        USUAL,
      );
      expect(api.startCustomJob).toHaveBeenNthCalledWith(
        2,
        "/clips/b.mov",
        expect.anything(),
        template,
        USUAL,
      );
      expect(api.startJob).not.toHaveBeenCalled();
    });
  });

  describe("pictures → animation", () => {
    const picture = (path: string): MediaInfo => ({
      ...PHOTO,
      path,
      fileName: path.split("/").pop() ?? path,
      sizeBytes: 2_000_000,
    });
    const PICS = ["/pics/cat 10.jpg", "/pics/cat 2.jpg", "/pics/cat 1.jpg"];

    beforeEach(() => {
      vi.mocked(api.probeFile).mockImplementation(async (path) =>
        path.endsWith(".mov") ? VIDEO : picture(path),
      );
      vi.mocked(api.startSequence).mockImplementation(async (paths, options) => ({
        id: 11,
        output: `/pics/cat.${options.format}`,
        inputs: paths.map(picture),
        command: "ffmpeg …",
      }));
    });

    async function openAnimation() {
      render(() => <App />);
      await ffmpegFound();
      openPaths(PICS);
      expect(await screen.findByRole("heading", { name: "3 pictures" })).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: /Make an animation/ }));
      return screen.findByRole("list", { name: "Pictures in order" });
    }

    const order = () =>
      within(screen.getByRole("list", { name: "Pictures in order" }))
        .getAllByRole("listitem")
        .map((li) => li.getAttribute("aria-label"));

    it("never offers to replace the pictures", async () => {
      localStorage.setItem(
        "catconvert.settings.v1",
        JSON.stringify({ ...DEFAULTS, output: "replaceInput", replaceConfirmed: true }),
      );
      resetStore();
      await openAnimation();
      const save = screen.getByRole("combobox", { name: "Save" });
      expect(save).toHaveDisplayValue("Next to the first picture");
      expect(within(save).queryByText("In place of the original")).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: /Make animation/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startSequence).toHaveBeenCalledWith(
        expect.any(Array),
        expect.objectContaining({ destination: { kind: "nextToInput" } }),
        expect.any(Object),
        USUAL,
      );
    });

    it("upscales the frame, lifting the default size cap", async () => {
      await openAnimation();
      fireEvent.change(screen.getByRole("combobox", { name: "Size" }), {
        target: { value: "x2" },
      });
      const animation = within(screen.getByRole("list", { name: "Animation" }));
      expect(animation.getByText("8064×6048")).toBeInTheDocument();
      expect(
        screen.getByText("That's 8064×6048: encoding will be slow and the file big."),
      ).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: /Make animation/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startSequence).toHaveBeenCalledWith(
        expect.any(Array),
        expect.objectContaining({ upscale: { factor: 2 }, maxSize: null, scaler: "auto" }),
        expect.any(Object),
        USUAL,
      );
    });

    it("makes one animation from dropped pictures", async () => {
      await openAnimation();
      // Natural filename order: 2 before 10.
      expect(order()).toEqual(["1. cat 1.jpg", "2. cat 2.jpg", "3. cat 10.jpg"]);
      expect(screen.getByRole("button", { name: "GIF" })).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByRole("button", { name: "WEBP" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "0.5 s" })).toHaveAttribute("aria-pressed", "true");
      expect(
        await screen.findByText("Saved as cat.gif next to the first picture."),
      ).toBeInTheDocument();
      expect(screen.getByRole("textbox", { name: "File name" })).toHaveAttribute(
        "placeholder",
        "cat",
      );
      expect(screen.getByRole("combobox", { name: "Size" })).toHaveDisplayValue("Auto (480 px)");

      fireEvent.click(screen.getByRole("button", { name: "0.25 s" }));
      fireEvent.click(screen.getByRole("button", { name: "Boomerang" }));
      fireEvent.click(screen.getByRole("button", { name: "Crop to fill" }));
      const animation = within(screen.getByRole("list", { name: "Animation" }));
      expect(animation.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
        "4 frames",
        "1 s loop",
        "640×480",
        "Boomerang",
      ]);
      expect(screen.getByText(/^About [\d.]+ (KB|MB) · /)).toBeInTheDocument();

      // Reorder with the keyboard, then leave one out.
      fireEvent.keyDown(screen.getByRole("listitem", { name: "3. cat 10.jpg" }), {
        key: "ArrowLeft",
      });
      expect(order()).toEqual(["1. cat 1.jpg", "2. cat 10.jpg", "3. cat 2.jpg"]);
      fireEvent.click(screen.getByRole("button", { name: "Leave out cat 2.jpg" }));
      expect(order()).toEqual(["1. cat 1.jpg", "2. cat 10.jpg"]);
      fireEvent.keyDown(screen.getByRole("listitem", { name: "1. cat 1.jpg" }), {
        key: "Delete",
      });
      expect(screen.getByText("An animation needs at least two pictures")).toBeInTheDocument();
      expect(order()).toHaveLength(2);

      fireEvent.click(screen.getByRole("button", { name: /Make animation/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startSequence).toHaveBeenCalledWith(
        ["/pics/cat 1.jpg", "/pics/cat 10.jpg"],
        expect.objectContaining({
          format: "gif",
          maxSize: null,
          metadata: "none",
          direction: "boomerang",
        }),
        { frameSecs: 0.25, fit: "crop" },
        USUAL,
      );
      expect(screen.getByText("2 PICTURES → GIF · Waiting")).toBeInTheDocument();

      applyProgress({
        jobId: 11,
        stage: "done",
        progress: 1,
        speed: null,
        outputBytes: 400_000,
        error: null,
      });
      expect(state.history[0]).toEqual(
        expect.objectContaining({ name: "cat.gif", pictures: 2, inputBytes: 4_000_000 }),
      );
      fireEvent.click(screen.getByRole("button", { name: "History" }));
      expect(await screen.findByText("2 pictures → cat.gif")).toBeInTheDocument();
    });

    it("shows a slideshow's CRF and average bitrate", async () => {
      await openAnimation();
      expect(document.querySelector(".quality-numbers")).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "MP4" }));
      expect(await screen.findByText(/^Video ~[\d.]+ (k|M)bps \(CRF 24\)$/)).toBeInTheDocument();
      // Slideshows stay at constant quality: no exact-bitrate mode.
      expect(screen.queryByRole("group", { name: "Set quality by" })).not.toBeInTheDocument();
    });

    it("converts each picture on its own when asked", async () => {
      render(() => <App />);
      await ffmpegFound();
      openPaths(["/pics/a.png", "/pics/b.png"]);
      fireEvent.click(await screen.findByRole("button", { name: /Convert each/ }));
      expect(await screen.findByRole("heading", { name: "a.png" })).toBeInTheDocument();
      expect(screen.getByText("Up next")).toBeInTheDocument();
      expect(api.startSequence).not.toHaveBeenCalled();
    });

    it("doesn't ask when the drop isn't only pictures", async () => {
      render(() => <App />);
      await ffmpegFound();
      openPaths([VIDEO.path, "/pics/a.png"]);
      expect(await screen.findByRole("heading", { name: "clip.mov" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /Make an animation/ })).not.toBeInTheDocument();
    });

    it("names the animation and asks before replacing a file", async () => {
      vi.mocked(api.sequenceOutputPreview).mockImplementation(
        async (_pictures, options, target) => ({
          path: `/pics/${target.name ?? "cat"}.${options.format}`,
          exists: target.name === "flipbook",
        }),
      );
      vi.mocked(api.askReplace).mockResolvedValueOnce("replace");
      await openAnimation();
      fireEvent.input(screen.getByRole("textbox", { name: "File name" }), {
        target: { value: "flipbook" },
      });
      expect(await screen.findByText(/The name is taken/)).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: /Make animation/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.askReplace).toHaveBeenCalledWith("flipbook.gif", "/pics");
      expect(api.startSequence).toHaveBeenCalledWith(
        expect.anything(),
        expect.anything(),
        expect.anything(),
        {
          folder: null,
          name: "flipbook",
          collision: "overwrite",
        },
      );
    });

    it("retries without pictures that were moved since", async () => {
      await openAnimation();
      fireEvent.click(screen.getByRole("button", { name: /Make animation/ }));
      await screen.findByRole("heading", { name: "Queue" });
      applyProgress({
        jobId: 11,
        stage: "failed",
        progress: 0.2,
        speed: null,
        outputBytes: null,
        error: "Disk full",
      });
      vi.mocked(api.missingFiles).mockResolvedValue(["/pics/cat 2.jpg"]);
      fireEvent.click(await screen.findByRole("button", { name: /Retry/ }));
      await waitFor(() => expect(api.startSequence).toHaveBeenCalledTimes(2));
      expect(api.startSequence).toHaveBeenLastCalledWith(
        ["/pics/cat 1.jpg", "/pics/cat 10.jpg"],
        expect.objectContaining({ format: "gif", direction: "forward" }),
        { frameSecs: 0.5, fit: "pad" },
        USUAL,
      );
      expect(
        screen.getByText("1 picture was moved or deleted, so it's made from the other 2"),
      ).toBeInTheDocument();
    });

    it("converts an animation again from History in the sequence editor", async () => {
      vi.mocked(api.missingFiles).mockImplementation(async () => []);
      await openAnimation();
      fireEvent.click(screen.getByRole("button", { name: "0.25 s" }));
      fireEvent.click(screen.getByRole("button", { name: "Boomerang" }));
      fireEvent.click(screen.getByRole("button", { name: /Make animation/ }));
      await screen.findByRole("heading", { name: "Queue" });
      applyProgress({
        jobId: 11,
        stage: "done",
        progress: 1,
        speed: null,
        outputBytes: 400_000,
        error: null,
      });
      expect(state.history[0]).toMatchObject({
        inputs: ["/pics/cat 1.jpg", "/pics/cat 2.jpg", "/pics/cat 10.jpg"],
        options: { format: "gif", direction: "boomerang" },
        sequence: { frameSecs: 0.25, fit: "pad" },
      });

      // One picture went missing since: the other two still make it.
      vi.mocked(api.missingFiles).mockImplementation(async (paths) =>
        paths.filter((p) => p === "/pics/cat 2.jpg"),
      );
      fireEvent.click(screen.getByRole("button", { name: "History" }));
      fireEvent.click(await screen.findByRole("button", { name: "Convert cat.gif again" }));
      await screen.findByRole("list", { name: "Pictures in order" });
      expect(order()).toEqual(["1. cat 1.jpg", "2. cat 10.jpg"]);
      expect(screen.getByRole("button", { name: "0.25 s" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      expect(screen.getByRole("button", { name: "Boomerang" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      expect(
        screen.getByText("1 picture was moved or deleted, so it's made from the other 2"),
      ).toBeInTheDocument();
    });

    it("opens the animation itself once its pictures are gone", async () => {
      await openAnimation();
      fireEvent.click(screen.getByRole("button", { name: /Make animation/ }));
      await screen.findByRole("heading", { name: "Queue" });
      applyProgress({
        jobId: 11,
        stage: "done",
        progress: 1,
        speed: null,
        outputBytes: 400_000,
        error: null,
      });
      vi.mocked(api.missingFiles).mockImplementation(async (paths) =>
        paths.filter((p) => p.endsWith(".jpg")),
      );
      vi.mocked(api.probeFile).mockImplementation(async (path) => ({
        ...VIDEO,
        path,
        fileName: "cat.gif",
        extension: "gif",
        kind: "video",
      }));
      fireEvent.click(screen.getByRole("button", { name: "History" }));
      const again = await screen.findByRole("button", { name: "Convert cat.gif again" });
      await waitFor(() =>
        expect(again).toHaveAttribute(
          "title",
          "The pictures are gone; opens the animation instead",
        ),
      );
      fireEvent.click(again);
      expect(await screen.findByRole("heading", { name: "cat.gif" })).toBeInTheDocument();
      expect(api.probeFile).toHaveBeenLastCalledWith("/pics/cat.gif");
      expect(
        screen.getByText("The pictures are gone, so the converted cat.gif opens instead"),
      ).toBeInTheDocument();
    });
  });

  it("adds and removes the Finder Quick Action in Settings", async () => {
    vi.mocked(api.quickActionStatus).mockResolvedValue({ supported: true, installed: false });
    vi.mocked(api.setQuickAction).mockImplementation(async (installed) => ({
      supported: true,
      installed,
    }));
    render(() => <App />);
    navigate("settings");

    const toggle = await screen.findByRole("switch", { name: "Finder Quick Action" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "true"));
    expect(api.setQuickAction).toHaveBeenLastCalledWith(true);
    expect(screen.getByText(/Quick Actions › Convert with CatConvert/)).toBeInTheDocument();

    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
    expect(api.setQuickAction).toHaveBeenLastCalledWith(false);
  });

  it("only offers the Quick Action where there is one", async () => {
    render(() => <App />);
    navigate("settings");
    expect(await screen.findByRole("heading", { name: "From other apps" })).toBeInTheDocument();
    await waitFor(() => expect(api.quickActionStatus).toHaveBeenCalled());
    expect(screen.queryByRole("switch", { name: "Finder Quick Action" })).not.toBeInTheDocument();
  });

  it("drags finished files out of History once the pointer moves", async () => {
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    applyProgress({
      jobId: 7,
      stage: "done",
      progress: 1,
      speed: null,
      outputBytes: 9,
      error: null,
    });
    fireEvent.click(screen.getByRole("button", { name: "History" }));
    const row = (await screen.findByText("clip.mp4")).closest("li") as HTMLElement;

    // A press on the folder button stays a click.
    fireEvent.pointerDown(screen.getByRole("button", { name: /Show clip.mp4 in folder/ }), {
      button: 0,
      clientX: 10,
      clientY: 10,
    });
    fireEvent.pointerMove(window, { clientX: 40, clientY: 10 });
    fireEvent.pointerUp(window);
    // So does a press that barely moves.
    fireEvent.pointerDown(row, { button: 0, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(window, { clientX: 12, clientY: 11 });
    fireEvent.pointerUp(window);
    expect(api.dragFiles).not.toHaveBeenCalled();

    fireEvent.pointerDown(row, { button: 0, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(window, { clientX: 30, clientY: 14 });
    expect(api.dragFiles).toHaveBeenCalledTimes(1);
    expect(api.dragFiles).toHaveBeenCalledWith(["/clips/clip.mp4"], expect.any(String));
    // Later moves belong to the native drag.
    fireEvent.pointerMove(window, { clientX: 60, clientY: 20 });
    expect(api.dragFiles).toHaveBeenCalledTimes(1);
  });

  it("marks history entries whose files were moved and can forget them", async () => {
    await openVideo();
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    applyProgress({
      jobId: 7,
      stage: "done",
      progress: 1,
      speed: null,
      outputBytes: 9,
      error: null,
    });
    vi.mocked(api.missingFiles).mockResolvedValue(["/clips/clip.mp4"]);

    fireEvent.click(screen.getByRole("button", { name: "History" }));
    expect(await screen.findByText(/Moved or deleted/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Show clip.mp4 in folder/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Forget moved (1)" }));
    expect(screen.getByText("A clean slate")).toBeInTheDocument();
    expect(state.history).toHaveLength(0);
  });

  describe("theme", () => {
    it("follows the system until Light or Dark is picked, and remembers the pick", async () => {
      render(() => <App />);
      await ffmpegFound();
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      const root = document.documentElement;
      expect(screen.getByRole("button", { name: "Same as system" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      expect(root).not.toHaveAttribute("data-theme");

      fireEvent.click(screen.getByRole("button", { name: "Dark" }));
      expect(root).toHaveAttribute("data-theme", "dark");
      expect(screen.getByRole("button", { name: "Dark" })).toHaveAttribute("aria-pressed", "true");
      expect(api.setWindowTheme).toHaveBeenLastCalledWith("dark");
      expect(JSON.parse(localStorage.getItem("catconvert.theme.v1") ?? "null")).toBe("dark");

      fireEvent.click(screen.getByRole("button", { name: "Light" }));
      expect(root).toHaveAttribute("data-theme", "light");
      expect(api.setWindowTheme).toHaveBeenLastCalledWith("light");

      // Read back at the next launch.
      resetStore();
      expect(state.theme).toBe("light");
      navigate("settings");

      fireEvent.click(screen.getByRole("button", { name: "Same as system" }));
      expect(root).not.toHaveAttribute("data-theme");
      expect(api.setWindowTheme).toHaveBeenLastCalledWith("system");
      expect(JSON.parse(localStorage.getItem("catconvert.theme.v1") ?? "null")).toBe("system");
    });

    it("redraws the waveform in the dark accent", async () => {
      await openSong();
      await waitFor(() =>
        expect(api.waveform).toHaveBeenLastCalledWith(SONG.path, 1200, 160, "#c93b63"),
      );
      setTheme("dark");
      await waitFor(() =>
        expect(api.waveform).toHaveBeenLastCalledWith(SONG.path, 1200, 160, "#ee7a9b"),
      );
      // The old picture stays up while the new one is drawn.
      expect(document.querySelector(".trim-wave")).toBeInTheDocument();
    });
  });

  describe("where files go", () => {
    it("saves into the suggested folder once chosen in Settings", async () => {
      render(() => <App />);
      await ffmpegFound();
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      const next = screen.getByRole("button", { name: "Next to the original" });
      expect(next).toHaveAttribute("aria-pressed", "true");

      fireEvent.click(screen.getByRole("button", { name: "In a folder" }));
      await waitFor(() =>
        expect(screen.getByRole("button", { name: "In a folder" })).toHaveAttribute(
          "aria-pressed",
          "true",
        ),
      );
      expect(api.prepareOutputFolder).toHaveBeenCalledWith("/Users/cat/Downloads/CatConvert");
      expect(screen.getByText("/Users/cat/Downloads/CatConvert")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("switch", { name: /Sort by kind/ }));
      expect(JSON.parse(localStorage.getItem("catconvert.settings.v1") ?? "{}")).toMatchObject({
        output: "folder",
        outputFolder: "/Users/cat/Downloads/CatConvert",
        outputByKind: true,
      });

      // Another folder from the picker.
      vi.mocked(api.pickFolder).mockResolvedValueOnce("/Volumes/Cats");
      fireEvent.click(screen.getByRole("button", { name: "Change…" }));
      await screen.findByText("/Volumes/Cats");

      openPaths([VIDEO.path]);
      await screen.findByRole("heading", { name: "clip.mov" });
      expect(screen.getByRole("combobox", { name: "Save" })).toHaveDisplayValue("In Cats");
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startJob).toHaveBeenCalledWith(
        VIDEO.path,
        expect.objectContaining({
          destination: { kind: "folder", path: "/Volumes/Cats", byKind: true },
        }),
        USUAL,
      );
    });

    it("doesn't use a folder files can't be saved in", async () => {
      vi.mocked(api.prepareOutputFolder).mockRejectedValueOnce("permission denied");
      render(() => <App />);
      await ffmpegFound();
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      fireEvent.click(screen.getByRole("button", { name: "In a folder" }));
      expect(await screen.findByText("Can't save there: Permission denied")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Next to the original" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      expect(state.settings.outputFolder).toBeNull();
    });

    it("asks before replacing originals for the first time", async () => {
      render(() => <App />);
      await ffmpegFound();
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      const replace = () => screen.getByRole("button", { name: "Replace the original" });

      fireEvent.click(replace());
      expect(screen.getByRole("alertdialog", { name: /Replace originals/ })).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      expect(replace()).toHaveAttribute("aria-pressed", "false");

      fireEvent.click(replace());
      fireEvent.click(screen.getByRole("button", { name: "Replace originals" }));
      await waitFor(() => expect(replace()).toHaveAttribute("aria-pressed", "true"));
      expect(state.settings).toMatchObject({ output: "replaceInput", replaceConfirmed: true });

      // Once agreed, it doesn't ask again.
      const next = screen.getByRole("button", { name: "Next to the original" });
      fireEvent.click(next);
      await waitFor(() => expect(next).toHaveAttribute("aria-pressed", "true"));
      fireEvent.click(replace());
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      await waitFor(() => expect(replace()).toHaveAttribute("aria-pressed", "true"));
    });

    it("replaces one file from the editor and remembers it in History", async () => {
      vi.mocked(api.startJob).mockImplementation(async (path, options) => ({
        id: 7,
        output: path.replace(/\.\w+$/, `.${options.format}`),
        replaces: options.destination?.kind === "replaceInput" ? path : null,
        inputs: [VIDEO],
        command: "ffmpeg …",
      }));
      await openVideo();
      const save = screen.getByRole("combobox", { name: "Save" });
      expect(save).toHaveDisplayValue("Next to the original");

      // The first time, it asks; the choice waits for the answer.
      fireEvent.change(save, { target: { value: "replaceInput" } });
      expect(screen.getByRole("alertdialog", { name: /Replace originals/ })).toBeInTheDocument();
      expect(save).toHaveDisplayValue("Next to the original");
      fireEvent.click(screen.getByRole("button", { name: "Replace originals" }));
      await waitFor(() => expect(save).toHaveDisplayValue("In place of the original"));
      expect(screen.getByText(/Replaces the original; it goes to the Trash/)).toBeInTheDocument();
      // Only this conversion: the default stays next to the original.
      expect(state.settings.output).toBe("nextToInput");

      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startJob).toHaveBeenCalledWith(
        VIDEO.path,
        expect.objectContaining({ destination: { kind: "replaceInput" } }),
        USUAL,
      );
      expect(screen.getByText("Replaces original")).toBeInTheDocument();
      applyProgress({
        jobId: 7,
        stage: "running",
        progress: 0.5,
        speed: 1,
        outputBytes: null,
        error: null,
      });
      expect(screen.getByText("replacing the original")).toBeInTheDocument();

      applyProgress({
        jobId: 7,
        stage: "done",
        progress: 1,
        speed: null,
        outputBytes: 9,
        output: "/clips/clip 2.mp4",
        replaced: { to: "renamed", path: "/clips/clip.orig.mov" },
        error: null,
      });
      expect(screen.getByText("clip 2.mp4")).toBeInTheDocument();
      expect(screen.getByText("Replaced original")).toBeInTheDocument();
      expect(
        screen.getByText("There's no Trash here, so the original was kept as clip.orig.mov"),
      ).toBeInTheDocument();
      expect(state.history[0]).toEqual(
        expect.objectContaining({
          name: "clip 2.mp4",
          output: "/clips/clip 2.mp4",
          replaced: true,
          keptAs: "/clips/clip.orig.mov",
        }),
      );

      fireEvent.click(screen.getByRole("button", { name: "History" }));
      expect(await screen.findByText("Replaced original")).toHaveAttribute(
        "title",
        "The original was kept as clip.orig.mov",
      );

      // Converting it again opens the converted file, which must not be
      // replaced in turn: where it goes follows Settings.
      vi.mocked(api.missingFiles).mockImplementation(async (paths) =>
        paths.filter((p) => p === VIDEO.path),
      );
      fireEvent.click(screen.getByRole("button", { name: "Convert clip 2.mp4 again" }));
      expect(await screen.findByRole("heading", { name: "clip 2.mp4" })).toBeInTheDocument();
      expect(screen.getByRole("combobox", { name: "Save" })).toHaveDisplayValue(
        "Next to the original",
      );
      expect(state.draft?.options.destination).toEqual({ kind: "nextToInput" });
    });
  });

  describe("converting again from History", () => {
    beforeEach(() => {
      vi.mocked(api.missingFiles).mockImplementation(async () => []);
    });

    /** Convert clip.mov to MP4 at quality 80, turned 90°, and open History. */
    async function convertedVideo() {
      await openVideo();
      fireEvent.input(screen.getByRole("slider", { name: "Quality" }), {
        target: { value: "80" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Rotate 90°" }));
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await screen.findByRole("heading", { name: "Queue" });
      applyProgress({
        jobId: 7,
        stage: "done",
        progress: 1,
        speed: null,
        outputBytes: 9,
        error: null,
      });
      fireEvent.click(screen.getByRole("button", { name: "History" }));
      return screen.findByText("clip.mp4");
    }

    const quality = () =>
      (screen.getByRole("slider", { name: "Quality" }) as HTMLInputElement).value;
    const changes = () =>
      within(screen.getByRole("list", { name: "Changes" }))
        .getAllByRole("listitem")
        .map((li) => li.textContent);

    it("remembers the inputs and settings of each conversion", async () => {
      await convertedVideo();
      expect(state.history[0]).toMatchObject({
        inputs: [VIDEO.path],
        options: { format: "mp4", quality: 80, rotate: 90 },
      });
      expect(state.history[0]).not.toHaveProperty("command");
      const saved = JSON.parse(localStorage.getItem("catconvert.history.v2") ?? "[]");
      expect(saved[0]).toMatchObject({ inputs: [VIDEO.path], options: { quality: 80 } });
    });

    it("opens the original with the old settings and converts a new file", async () => {
      await convertedVideo();
      fireEvent.click(screen.getByRole("button", { name: "Convert clip.mp4 again" }));
      expect(await screen.findByRole("heading", { name: "clip.mov" })).toBeInTheDocument();
      expect(api.probeFile).toHaveBeenLastCalledWith(VIDEO.path);
      expect(screen.getByRole("button", { name: "MP4" })).toHaveAttribute("aria-pressed", "true");
      expect(quality()).toBe("80");
      expect(changes()).toContain("Turned 90°");

      fireEvent.click(screen.getByRole("button", { name: "WEBM" }));
      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startJob).toHaveBeenLastCalledWith(
        VIDEO.path,
        expect.objectContaining({ format: "webm", quality: 80, rotate: 90 }),
        USUAL,
      );
    });

    it("opens the converted file instead when the original is gone", async () => {
      vi.mocked(api.missingFiles).mockImplementation(async (paths) =>
        paths.filter((p) => p === VIDEO.path),
      );
      await convertedVideo();
      const again = await screen.findByRole("button", { name: "Convert clip.mp4 again" });
      await waitFor(() =>
        expect(again).toHaveAttribute(
          "title",
          "The original is gone; opens the converted file instead",
        ),
      );
      fireEvent.click(again);
      expect(await screen.findByRole("heading", { name: "clip.mp4" })).toBeInTheDocument();
      expect(api.probeFile).toHaveBeenLastCalledWith("/clips/clip.mp4");
      expect(
        screen.getByText("clip.mov is gone, so the converted clip.mp4 opens instead"),
      ).toBeInTheDocument();
      // The format and quality carry over; the turn is already in the file.
      expect(screen.getByRole("button", { name: "MP4" })).toHaveAttribute("aria-pressed", "true");
      expect(quality()).toBe("80");
      expect(screen.queryByRole("list", { name: "Changes" })?.textContent ?? "").not.toContain(
        "Turned",
      );
    });

    it("says why it can't when the original and the converted file are gone", async () => {
      vi.mocked(api.missingFiles).mockImplementation(async (paths) => paths);
      const name = await convertedVideo();
      const again = screen.getByRole("button", { name: "Convert clip.mp4 again" });
      await waitFor(() => expect(again).toBeDisabled());
      expect(again).toHaveAttribute("title", "The original and the converted file are both gone");

      fireEvent.contextMenu(name);
      const menu = within(await screen.findByRole("menu", { name: "clip.mp4" }));
      const item = menu.getByRole("menuitem", { name: /Convert again/ });
      expect(item).toBeDisabled();
      expect(item).toHaveTextContent("The original and the converted file are both gone");
      expect(menu.queryByText("Convert again with")).not.toBeInTheDocument();
      expect(menu.getByRole("menuitem", { name: /Show in folder/ })).toBeDisabled();
      fireEvent.keyDown(menu.getByRole("menuitem", { name: /Convert again/ }), {
        key: "Escape",
      });
      expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    });

    it("converts again with a recipe from the right-click menu", async () => {
      const name = await convertedVideo();
      fireEvent.contextMenu(name);
      const menu = within(await screen.findByRole("menu", { name: "clip.mp4" }));
      expect(menu.getByText("Convert again with")).toBeInTheDocument();
      // Only recipes for videos; the first item has focus.
      expect(menu.getByRole("menuitem", { name: /Convert again/ })).toHaveFocus();
      expect(menu.getByRole("menuitem", { name: "Make a GIF" })).toBeInTheDocument();
      fireEvent.keyDown(menu.getByRole("menuitem", { name: /Convert again/ }), { key: "End" });
      expect(menu.getByRole("menuitem", { name: "Show in folder" })).toHaveFocus();

      fireEvent.click(menu.getByRole("menuitem", { name: "Web friendly" }));
      expect(screen.queryByRole("menu")).not.toBeInTheDocument();
      expect(await screen.findByRole("heading", { name: "clip.mov" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "WEBM" })).toHaveAttribute("aria-pressed", "true");
      expect(quality()).toBe("50");

      fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
      await screen.findByRole("heading", { name: "Queue" });
      expect(api.startJob).toHaveBeenLastCalledWith(
        VIDEO.path,
        expect.objectContaining({ format: "webm", quality: 50 }),
        USUAL,
      );
      applyProgress({
        jobId: 7,
        stage: "done",
        progress: 1,
        speed: null,
        outputBytes: 5,
        error: null,
      });
      // Both jobs share the mocked id 7; the second one finishes here.
      expect(state.jobs.at(-1)?.recipe).toBe("Web friendly");
    });

    it("asks to finish what's open in the editor first", async () => {
      await convertedVideo();
      openPaths([SONG.path]);
      await screen.findByRole("heading", { name: "purr.wav" });
      fireEvent.click(screen.getByRole("button", { name: "History" }));
      fireEvent.click(await screen.findByRole("button", { name: "Convert clip.mp4 again" }));
      expect(await screen.findByText("Finish what's open in the editor first")).toBeInTheDocument();
      expect(state.draft?.info.path).toBe(SONG.path);
    });

    it("moves version 1 history over and still shows its entries", async () => {
      const old = {
        id: "1-3",
        name: "song.mp3",
        output: "/music/song.mp3",
        format: "mp3",
        kind: "audio",
        inputBytes: 4_000_000,
        outputBytes: 1_000_000,
        finishedAt: Date.now() - 60_000,
      };
      localStorage.setItem("catconvert.history.v1", JSON.stringify([old, "junk", null]));
      resetStore();
      expect(state.history).toEqual([old]);
      expect(localStorage.getItem("catconvert.history.v1")).toBeNull();
      expect(JSON.parse(localStorage.getItem("catconvert.history.v2") ?? "null")).toEqual([
        old,
        "junk",
        null,
      ]);

      render(() => <App />);
      navigate("history");
      const name = await screen.findByText("song.mp3");
      expect(screen.getByRole("button", { name: "Show song.mp3 in folder" })).toBeEnabled();
      expect(screen.queryByRole("button", { name: /again/ })).not.toBeInTheDocument();
      fireEvent.contextMenu(name);
      const menu = within(await screen.findByRole("menu", { name: "song.mp3" }));
      expect(menu.getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
        "Show in folder",
      ]);
    });

    it("keeps an entry whose stored settings are broken, without the action", () => {
      const broken = {
        id: "2-4",
        name: "clip.mp4",
        output: "/clips/clip.mp4",
        format: "mp4",
        kind: "video",
        inputBytes: 1,
        outputBytes: 1,
        finishedAt: 1,
        inputs: [VIDEO.path],
        options: { quality: 80 },
      };
      localStorage.setItem("catconvert.history.v2", JSON.stringify([broken]));
      resetStore();
      const { inputs: _inputs, options: _options, ...shown } = broken;
      expect(state.history).toEqual([shown]);
    });
  });
});

describe("upscaling", () => {
  const changes = () =>
    within(screen.getByRole("list", { name: "Changes" }))
      .getAllByRole("listitem")
      .map((li) => li.textContent);
  const sizeOptions = () =>
    within(screen.getByRole("combobox", { name: "Size" }))
      .getAllByRole("option")
      .map((option) => option.textContent);

  async function open(info: MediaInfo) {
    vi.mocked(api.probeFile).mockResolvedValue(info);
    render(() => <App />);
    await ffmpegFound();
    openPaths([info.path]);
    return screen.findByRole("heading", { name: info.fileName });
  }

  it("doubles a 640×480 photo with Lanczos", async () => {
    await open({ ...PHOTO, width: 640, height: 480 });
    expect(sizeOptions()).toEqual([
      "Original (480 px)",
      "360 px",
      "240 px",
      "2×",
      "3×",
      "4×",
      "720 px",
      "1080 px",
      "1440 px",
      "2160 px",
      "Custom…",
    ]);
    expect(screen.queryByRole("combobox", { name: "Scaler" })).not.toBeInTheDocument();

    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "x2" } });
    expect(changes()).toContain("2× · 1280×960");
    expect(screen.getByRole("combobox", { name: "Scaler" })).toHaveDisplayValue("Lanczos (sharp)");
    expect(screen.getByText("Upscaling makes it bigger, not sharper.")).toBeInTheDocument();

    // A shrink size clears the upscale, and back.
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "360" } });
    expect(screen.queryByRole("combobox", { name: "Scaler" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "x2" } });
    fireEvent.change(screen.getByRole("combobox", { name: "Scaler" }), {
      target: { value: "spline" },
    });

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      PHOTO.path,
      expect.objectContaining({ upscale: { factor: 2 }, maxSize: null, scaler: "spline" }),
      USUAL,
    );
  });

  it("offers AI upscaling for pictures once it is installed", async () => {
    vi.mocked(api.ffmpegStatus).mockResolvedValue({ ...status(), aiModels: ["general", "anime"] });
    await open({ ...PHOTO, width: 640, height: 480 });
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "x2" } });
    const scaler = () => screen.getByRole("combobox", { name: "Scaler" });
    expect(
      within(scaler())
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual([
      "Lanczos (sharp)",
      "Spline",
      "Bicubic (softer)",
      "AI: photos",
      "AI: drawings and anime",
    ]);
    expect(screen.queryByRole("button", { name: /AI upscaling/ })).not.toBeInTheDocument();

    // One click picks the model to start with, without a look through the menu.
    fireEvent.click(screen.getByRole("button", { name: "Add detail with AI" }));
    expect(scaler()).toHaveDisplayValue("AI: photos");
    expect(screen.queryByRole("button", { name: "Add detail with AI" })).not.toBeInTheDocument();
    fireEvent.change(scaler(), { target: { value: "spline" } });

    fireEvent.change(scaler(), { target: { value: "ai:general" } });
    expect(scaler()).toHaveDisplayValue("AI: photos");
    expect(
      await screen.findByText(/Takes about 2 s to 9 s on a graphics card/),
    ).toBeInTheDocument();
    expect(api.aiEstimate).toHaveBeenLastCalledWith(
      expect.objectContaining({ path: PHOTO.path }),
      expect.objectContaining({ ai: "general", upscale: { factor: 2 } }),
    );
    // A classic scaler turns AI off again.
    fireEvent.change(scaler(), { target: { value: "spline" } });
    expect(scaler()).toHaveDisplayValue("Spline");
    expect(screen.getByText("Upscaling makes it bigger, not sharper.")).toBeInTheDocument();

    fireEvent.change(scaler(), { target: { value: "ai:anime" } });
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      PHOTO.path,
      expect.objectContaining({ upscale: { factor: 2 }, ai: "anime" }),
      USUAL,
    );
  });

  it("points to Settings while AI upscaling isn't installed", async () => {
    await open({ ...PHOTO, width: 640, height: 480 });
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "x2" } });
    expect(
      within(screen.getByRole("combobox", { name: "Scaler" }))
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["Lanczos (sharp)", "Spline", "Bicubic (softer)"]);
    fireEvent.click(screen.getByRole("button", { name: "Add detail with AI upscaling" }));
    // Straight to its card, which is halfway down Settings.
    const heading = await screen.findByRole("heading", { name: "AI upscaling" });
    expect(heading).toHaveFocus();
    expect(state.settingsTarget).toBeNull();
    expect(screen.getByText("Not installed")).toBeInTheDocument();
  });

  it("keeps videos to classic upscaling", async () => {
    vi.mocked(api.ffmpegStatus).mockResolvedValue({ ...status(), aiModels: ["general"] });
    await open({ ...VIDEO, width: 1280, height: 720 });
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), {
      target: { value: "s1080" },
    });
    expect(
      within(screen.getByRole("combobox", { name: "Scaler" }))
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["Lanczos (sharp)", "Spline", "Bicubic (softer)"]);
    expect(screen.queryByRole("button", { name: /Add detail with AI/ })).not.toBeInTheDocument();
  });

  it("turns a 720p video into 1080p", async () => {
    await open({ ...VIDEO, width: 1280, height: 720 });
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), {
      target: { value: "s1080" },
    });
    expect(screen.getByRole("combobox", { name: "Size" })).toHaveDisplayValue("1080p");
    expect(changes()).toContain("Upscaled to 1080p");
    expect(screen.queryByRole("status")).not.toHaveTextContent(/encoding will be slow/);

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ upscale: { size: 1080 }, maxSize: null, scaler: "auto" }),
      USUAL,
    );
  });

  it("warns about huge results and keeps within what the format can hold", async () => {
    await open({ ...VIDEO, width: 3840, height: 2160 });
    // MP4 stops at 8192 px on the long side, so 4K only doubles.
    expect(sizeOptions().slice(-2)).toEqual(["2×", "Custom…"]);
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "x2" } });
    expect(
      screen.getByText("That's 7680×4320: encoding will be slow and the file big."),
    ).toBeInTheDocument();

    // A custom short side is clamped to what fits, with a word why.
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), {
      target: { value: "custom" },
    });
    const field = screen.getByRole("textbox", { name: "Short side" });
    expect(field).toHaveValue("4320");
    fireEvent.change(field, { target: { value: "9000" } });
    expect(field).toHaveValue("4608");
    expect(screen.getByText("4608 px is as big as this file can go.")).toBeInTheDocument();
    expect(screen.getByText("Makes it 8192×4608")).toBeInTheDocument();
    fireEvent.change(field, { target: { value: "100" } });
    expect(field).toHaveValue("2162");

    // GIFs can go further; back in MP4 it would be too big to write.
    fireEvent.click(screen.getByRole("button", { name: "GIF" }));
    expect(sizeOptions()).toContain("4×");
    fireEvent.change(screen.getByRole("combobox", { name: "Size" }), { target: { value: "x4" } });
    expect(screen.getByText(/That's 15360×8640: encoding will be slow/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "MP4" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "That's 15360×8640, too big for MP4, which stops at 8192 px on the long side.",
    );
    expect(screen.getByRole("button", { name: /Start converting/ })).toBeDisabled();
  });

  it("loads settings saved before upscaling with the defaults", () => {
    const entry = {
      id: "3-1",
      name: "clip.mp4",
      output: "/clips/clip.mp4",
      format: "mp4",
      kind: "video",
      inputBytes: 1,
      outputBytes: 1,
      finishedAt: 1,
      inputs: [VIDEO.path],
      options: { format: "mp4", quality: 60, maxSize: 720 },
    };
    localStorage.setItem("catconvert.history.v2", JSON.stringify([entry]));
    const recipe = {
      id: "custom-2",
      title: "Bigger",
      options: { format: "png", upscale: { factor: "lots" }, scaler: "nearest" },
    };
    localStorage.setItem("catconvert.recipes.v1", JSON.stringify([recipe]));
    resetStore();
    expect(state.history[0]?.options).toEqual({
      format: "mp4",
      quality: 60,
      maxSize: 720,
      upscale: null,
      scaler: "auto",
    });
    expect(state.recipes[0]?.options).toEqual({ format: "png", upscale: null, scaler: "auto" });
  });
});

describe("Before and after", () => {
  it("shows the original next to the converted picture, and remembers it", async () => {
    await openPhoto();
    fireEvent.click(screen.getByRole("button", { name: "JPG" }));
    const toggle = screen.getByRole("button", { name: "Compare with the original" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("region", { name: "Before and after" })).not.toBeInTheDocument();

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    const view = within(await screen.findByRole("region", { name: "Before and after" }));
    await waitFor(() => expect(view.getByAltText("cat.jpg converted")).toBeInTheDocument());
    await waitFor(() =>
      expect(view.getByAltText("Original of cat.jpg")).toHaveAttribute(
        "src",
        expect.stringContaining("original-"),
      ),
    );
    // Both halves at preview size, from the editor's settings.
    const [info, options, , at, limit] = sampleCalls("converted")[0] ?? [];
    expect(info?.path).toBe(PHOTO.path);
    expect(options?.format).toBe("jpg");
    expect([at, limit]).toEqual([0, 1080]);
    expect(sampleCalls("original")).toHaveLength(1);
    expect(view.getByText("Original · 48.0 MB")).toBeInTheDocument();
    expect(view.getByText(/^JPG q60 · ~[\d.]+ [KM]B$/)).toBeInTheDocument();
    // Remembered for the next file and the next launch.
    expect(state.settings.compare).toBe(true);
    expect(JSON.parse(localStorage.getItem("catconvert.settings.v1") ?? "{}")).toMatchObject({
      compare: true,
    });

    // A new quality is encoded again; the original stays as it was.
    fireEvent.input(screen.getByRole("slider", { name: "Quality" }), { target: { value: "30" } });
    await waitFor(() => expect(sampleCalls("converted")).toHaveLength(2));
    expect(sampleCalls("converted")[1]?.[1].quality).toBe(30);
    expect(sampleCalls("original")).toHaveLength(1);
    await waitFor(() => expect(view.getByText(/ q30 · /)).toBeInTheDocument());

    // Zooming in renders full-size samples, whose size is exact.
    fireEvent.click(screen.getByRole("button", { name: "100%" }));
    expect(screen.getByRole("button", { name: "100%" })).toHaveAttribute("aria-pressed", "true");
    await waitFor(() => expect(sampleCalls("converted").at(-1)?.[4]).toBeNull());
    await waitFor(() => expect(sampleCalls("original").at(-1)?.[4]).toBeNull());
    await waitFor(() => expect(view.getByText(/ q30 · 612 KB$/)).toBeInTheDocument());

    // C turns it off again: the single view is just as it was.
    fireEvent.keyDown(window, { key: "c" });
    expect(screen.queryByRole("region", { name: "Before and after" })).not.toBeInTheDocument();
    expect(screen.getByAltText("Preview of cat.jpg")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Rotate 90°" })).toBeInTheDocument();
    expect(state.settings.compare).toBe(false);
  });

  it("opens files compared when it was left on", async () => {
    updateSettings({ compare: true, compareLayout: "side" });
    await openPhoto();
    const view = within(await screen.findByRole("region", { name: "Before and after" }));
    await waitFor(() => expect(view.getByAltText("cat.jpg converted")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Side by side" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    // Side by side, the preview takes the editor's whole width.
    expect(document.querySelector(".file-grid")).toHaveClass("compare-wide");
    fireEvent.click(screen.getByRole("button", { name: "Wipe" }));
    expect(state.settings.compareLayout).toBe("wipe");
    expect(document.querySelector(".file-grid")).not.toHaveClass("compare-wide");
    expect(view.getByRole("slider", { name: /Divider/ })).toBeInTheDocument();
  });

  it("compares the frame on screen of a video and wipes with the keyboard", async () => {
    await openVideo();
    fireEvent.keyDown(window, { key: "c" });
    await waitFor(() => expect(sampleCalls("converted")).toHaveLength(1));
    // The preview opens at 1 s.
    expect(sampleCalls("converted")[0]?.[3]).toBe(1);
    expect(sampleCalls("original")[0]?.[3]).toBe(1);

    fireEvent.click(screen.getByRole("button", { name: "Next frame" }));
    await waitFor(() => expect(sampleCalls("converted")).toHaveLength(2));
    expect(Math.ceil((sampleCalls("converted")[1]?.[3] ?? 0) * 30)).toBe(31);
    expect(sampleCalls("original")).toHaveLength(2);

    const divider = screen.getByRole("slider", { name: /Divider/ });
    expect(divider).toHaveAttribute("aria-valuenow", "50");
    fireEvent.keyDown(divider, { key: "ArrowRight" });
    expect(divider).toHaveAttribute("aria-valuenow", "52");
    fireEvent.keyDown(divider, { key: "Home" });
    expect(divider).toHaveAttribute("aria-valuenow", "0");
    // Keys on the divider don't step frames.
    expect(sampleCalls("converted")).toHaveLength(2);
  });

  it("says when a preview couldn't be made", async () => {
    vi.mocked(api.compareSample).mockImplementation(async (_info, _options, side) => {
      if (side === "converted") throw "this ffmpeg can't write AVIF";
      return { url: "asset://localhost/samples/original.png", bytes: null, exact: false };
    });
    await openPhoto();
    fireEvent.click(screen.getByRole("button", { name: "Compare with the original" }));
    expect(
      await screen.findByText("Couldn't make the preview: this ffmpeg can't write AVIF"),
    ).toBeInTheDocument();
  });

  it("flips between the halves on phones", async () => {
    const matchMedia = window.matchMedia;
    window.matchMedia = ((query: string) => ({
      matches: query.includes("max-width"),
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
    })) as unknown as typeof window.matchMedia;
    try {
      await openPhoto();
      fireEvent.click(screen.getByRole("button", { name: "Compare with the original" }));
      const view = within(await screen.findByRole("region", { name: "Before and after" }));
      // No room to split: one half at a time, the converted one first.
      expect(screen.queryByRole("button", { name: "Side by side" })).not.toBeInTheDocument();
      expect(view.queryByRole("slider")).not.toBeInTheDocument();
      const flip = await view.findByRole("button", { name: /^PNG · / });
      expect(flip).toHaveAttribute("aria-pressed", "false");
      fireEvent.click(flip);
      expect(view.getByRole("button", { name: /^Original/ })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    } finally {
      window.matchMedia = matchMedia;
    }
  });

  it("isn't offered for sound or an edited command", async () => {
    await openSong();
    expect(
      screen.queryByRole("button", { name: "Compare with the original" }),
    ).not.toBeInTheDocument();
    fireEvent.keyDown(window, { key: "c" });
    expect(api.compareSample).not.toHaveBeenCalled();
  });

  it("compares a finished file from the Queue and History with its original", async () => {
    vi.mocked(api.startJob).mockResolvedValue({
      id: 7,
      output: "/photos/cat.avif",
      inputs: [PHOTO],
      command: "ffmpeg …",
    });
    await openPhoto();
    fireEvent.click(screen.getByRole("button", { name: "Rotate 90°" }));
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    applyProgress({
      jobId: 7,
      stage: "done",
      progress: 1,
      speed: null,
      outputBytes: 610_000,
      error: null,
    });

    fireEvent.click(await screen.findByRole("button", { name: "Compare" }));
    const dialog = within(await screen.findByRole("dialog", { name: "cat.avif" }));
    await waitFor(() => expect(api.compareFile).toHaveBeenCalledTimes(1));
    expect(api.compareFile).toHaveBeenCalledWith("/photos/cat.avif", 0, 1080, false);
    // The original is turned like the result, so only the encoding differs.
    await waitFor(() => expect(sampleCalls("original")).toHaveLength(1));
    expect(sampleCalls("original")[0]?.[1]).toMatchObject({ rotate: 90 });
    expect(dialog.getByText(/ · 610 KB$/)).toBeInTheDocument();
    expect(dialog.getByText("Original · 48.0 MB")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "History" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Compare cat.avif with the original" }),
    );
    expect(await screen.findByRole("dialog", { name: "cat.avif" })).toBeInTheDocument();
    expect(api.probeFile).toHaveBeenLastCalledWith(PHOTO.path);
    await waitFor(() => expect(api.compareFile).toHaveBeenCalledTimes(2));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
