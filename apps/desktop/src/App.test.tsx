import { fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { FfmpegStatus, MediaInfo, OutputFormat } from "./lib/api";
import * as api from "./lib/api";
import { applyProgress, navigate, openPaths, resetStore, state } from "./lib/store";

vi.mock("./lib/api", () => ({
  inTauri: false,
  ffmpegStatus: vi.fn(),
  relocateFfmpeg: vi.fn(),
  probeFile: vi.fn(),
  previewFrame: vi.fn(async () => "blob:frame"),
  waveform: vi.fn(async () => "blob:wave"),
  previewCommand: vi.fn(async () => "ffmpeg -i clip.mov clip.mp4"),
  previewSequenceCommand: vi.fn(async () => "ffmpeg -f concat -i list.txt cat.gif"),
  saveFrame: vi.fn(),
  checkCommand: vi.fn(async (_info: unknown, _options: unknown, text: string) => text),
  fillCommand: vi.fn(),
  commandTemplate: vi.fn(),
  startCustomJob: vi.fn(),
  startJob: vi.fn(),
  startSequence: vi.fn(),
  cancelJob: vi.fn(async () => true),
  pickFiles: vi.fn(async () => []),
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
  NO_JXL: {},
}));

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
    videoCodecs: found ? ["h264", "vp9", "av1"] : [],
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
  resetStore();
  vi.clearAllMocks();
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
    expect(screen.getByText("clip.webm")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(VIDEO.path, {
      format: "webm",
      trimStart: 2,
      trimEnd: null,
      maxSize: 720,
      fps: null,
      rotate: 90,
      flip: false,
      crop: null,
      quality: 60,
      keepAudio: false,
      videoCodec: null,
      jxl: { lossless: false, effort: 7, progressive: false },
      sound: { normalize: false, fadeIn: 0, fadeOut: 0 },
      metadata: "noLocation",
      speed: 1,
      direction: "forward",
    });

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
    expect(screen.queryByRole("group", { name: "Video codec" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "MKV" }));
    expect(codecs().getByRole("button", { name: "H.264" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(codecs().getByRole("button", { name: "AV1" }));

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      VIDEO.path,
      expect.objectContaining({ format: "mkv", videoCodec: "av1" }),
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
    expect(screen.getByText("cat.jxl")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      PHOTO.path,
      expect.objectContaining({
        format: "jxl",
        rotate: 90,
        jxl: { lossless: false, effort: 9, progressive: true },
      }),
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
    );
    expect(api.startJob).toHaveBeenNthCalledWith(
      2,
      "/clips/b.mov",
      expect.objectContaining({ format: "webm", trimStart: null }),
    );
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
      expect(api.startCustomJob).toHaveBeenLastCalledWith(VIDEO.path, expect.anything(), edited);
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
      );
      expect(api.startCustomJob).toHaveBeenNthCalledWith(
        2,
        "/clips/b.mov",
        expect.anything(),
        template,
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

    it("makes one animation from dropped pictures", async () => {
      await openAnimation();
      // Natural filename order: 2 before 10.
      expect(order()).toEqual(["1. cat 1.jpg", "2. cat 2.jpg", "3. cat 10.jpg"]);
      expect(screen.getByRole("button", { name: "GIF" })).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByRole("button", { name: "WEBP" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "0.5 s" })).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByText("cat.gif")).toBeInTheDocument();
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
      );
      expect(
        screen.getByText("1 picture was moved or deleted, so it's made from the other 2"),
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
});
