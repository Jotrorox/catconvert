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
  startJob: vi.fn(),
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
    formats: ALL.map((format) => ({ format, available: found && format !== "webp" })),
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
  reconstructsJpeg: false,
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
};

const PACKED: MediaInfo = {
  ...PHOTO,
  path: "/photos/cat.jxl",
  fileName: "cat.jxl",
  extension: "jxl",
  videoCodec: "jpegxl",
  reconstructsJpeg: true,
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
};

beforeEach(() => {
  localStorage.clear();
  resetStore();
  vi.clearAllMocks();
  vi.mocked(api.ffmpegStatus).mockResolvedValue(status());
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
    input: VIDEO,
    command: "ffmpeg …",
  }));
});

async function openVideo() {
  render(() => <App />);
  await screen.findByText("ffmpeg 7.1");
  openPaths([VIDEO.path]);
  return screen.findByRole("heading", { name: "clip.mov" });
}

describe("CatConvert", () => {
  it("starts on Convert and reports the ffmpeg it found", async () => {
    render(() => <App />);
    expect(screen.getByRole("heading", { name: "Drop files on the kitty" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Convert" })).toHaveAttribute("aria-current", "page");
    expect(await screen.findByText("ffmpeg 7.1")).toBeInTheDocument();
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
      quality: 60,
      keepAudio: false,
      jxl: { lossless: false, effort: 7, progressive: false },
      keepMetadata: true,
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
      return { id: 9, output: `${path}.${options.format}`, input: VIDEO, command: "" };
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
    await screen.findByText("ffmpeg 7.1");
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
    await screen.findByText("ffmpeg 7.1");
    fireEvent.click(screen.getByRole("button", { name: /Make a GIF/ }));
    await screen.findByRole("heading", { name: "clip.mov" });
    expect(screen.getByRole("button", { name: "GIF" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("combobox", { name: "Size" })).toHaveValue("480");
    expect(screen.queryByRole("switch", { name: "Keep sound" })).not.toBeInTheDocument();
  });

  it("repacks a JPEG into lossless JPEG XL with libjxl settings", async () => {
    render(() => <App />);
    await screen.findByText("ffmpeg 7.1");
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

  it("restores the original JPEG from a repacked JPEG XL", async () => {
    render(() => <App />);
    await screen.findByText("ffmpeg 7.1");
    openPaths([PACKED.path]);
    await screen.findByRole("heading", { name: "cat.jxl" });
    expect(screen.getByText(/4032×3024 · JPEG XL/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "JPG" }));
    expect(screen.getByText("Restores the original JPEG, bit for bit.")).toBeInTheDocument();
  });

  it("offers only sound formats and no picture tools for audio", async () => {
    render(() => <App />);
    await screen.findByText("ffmpeg 7.1");
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

  it("starts files from the defaults chosen in Settings and remembers them", async () => {
    render(() => <App />);
    await screen.findByText("ffmpeg 7.1");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.change(screen.getByRole("combobox", { name: "Videos become" }), {
      target: { value: "mkv" },
    });
    fireEvent.input(screen.getByRole("slider", { name: "Quality" }), { target: { value: "85" } });
    fireEvent.click(screen.getByRole("switch", { name: "Keep metadata" }));
    expect(JSON.parse(localStorage.getItem("catconvert.settings.v1") ?? "{}")).toEqual({
      formats: { video: "mkv", audio: null, image: null },
      quality: 85,
      keepMetadata: false,
    });

    openPaths([VIDEO.path]);
    await screen.findByRole("heading", { name: "clip.mov" });
    expect(screen.getByRole("button", { name: "MKV" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("slider", { name: "Quality" })).toHaveValue("85");
    expect(screen.getByRole("switch", { name: "Keep metadata" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("strips metadata on request, which rules out the JPEG repack", async () => {
    render(() => <App />);
    await screen.findByText("ffmpeg 7.1");
    openPaths([PHOTO.path]);
    await screen.findByRole("heading", { name: "cat.jpg" });
    fireEvent.click(screen.getByRole("button", { name: "JXL" }));
    expect(screen.getByText(/Repacks the JPEG/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Keep metadata" }));
    expect(screen.getByText(/every pixel and sample stays put/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Start converting/ }));
    await screen.findByRole("heading", { name: "Queue" });
    expect(api.startJob).toHaveBeenCalledWith(
      PHOTO.path,
      expect.objectContaining({ format: "jxl", keepMetadata: false }),
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
    await screen.findByText("ffmpeg 7.1");
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
