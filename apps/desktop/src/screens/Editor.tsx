import {
  type Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  Index,
  Match,
  on,
  onCleanup,
  Show,
  Switch,
} from "solid-js";
import { CatPeek } from "../components/Cat";
import { CodecField } from "../components/CodecField";
import { CropOverlay } from "../components/CropOverlay";
import { Icon } from "../components/Icon";
import { QualityField } from "../components/QualityField";
import { TrimBar } from "../components/TrimBar";
import {
  type Aspect,
  type ConvertOptions,
  checkCommand,
  loudness,
  type MediaInfo,
  previewCommand,
  type QualitySummary,
  qualitySummary,
} from "../lib/api";
import {
  ASPECTS,
  cropBox,
  cropLabel,
  keptArea,
  mirrorCrop,
  type Size,
  sameAspect,
  turnCrop,
  turnedSize,
} from "../lib/crop";
import { describeEstimate, describeQuality, estimateVideoBitrate } from "../lib/estimate";
import {
  baseName,
  formatBitrate,
  formatBytes,
  formatClock,
  formatSpan,
  parseClock,
} from "../lib/format";
import {
  autoLimits,
  CODEC_LABELS,
  changesPlayback,
  codecOf,
  DIRECTIONS,
  describeStreams,
  FADE_CHOICES,
  fadeSpans,
  formatKind,
  fpsChoices,
  groupsFor,
  isAnimatedImage,
  isLossless,
  JXL_EFFORTS,
  keepsLocation,
  keepsMetadata,
  label,
  locationLine,
  METADATA_CHOICES,
  outputHasSound,
  outputSeconds,
  repacksJpeg,
  restoresJpeg,
  SPEEDS,
  shortSide,
  sizeChoices,
  sizeLabel,
  speedLabel,
} from "../lib/formats";
import { FALLBACK_FPS, stepFrame } from "../lib/frames";
import {
  createPlayer,
  normalizeGain,
  type PlayRange,
  playbackType,
  soundGain,
} from "../lib/player";
import { createFilmstrip, createFrame, createWaveform } from "../lib/preview";
import {
  canEncodeHevcInHardware,
  canRead,
  closeEditor,
  convertAll,
  convertDraft,
  convertEach,
  type Draft,
  isAvailable,
  isCodecAvailable,
  makeAnimation,
  saveFrame,
  saveRecipe,
  setCustomCommand,
  skipDraft,
  state,
  updateOptions,
} from "../lib/store";
import { SequenceEditor } from "./SequenceEditor";

const FILMSTRIP_FRAMES = 10;

function describe(info: MediaInfo): string {
  const parts: string[] = [];
  if (info.width && info.height) parts.push(`${info.width}×${info.height}`);
  if (info.durationSecs) parts.push(formatClock(info.durationSecs));
  if (info.videoCodec?.startsWith("jpegxl")) parts.push("JPEG XL");
  else {
    // Codecs with their bitrates, to compare with the quality setting.
    const streams = describeStreams(info);
    if (streams) parts.push(streams);
  }
  parts.push(formatBytes(info.sizeBytes));
  return parts.join(" · ");
}

function outputName(info: MediaInfo, options: ConvertOptions): string {
  const name = info.fileName;
  const dot = name.lastIndexOf(".");
  return `${dot > 0 ? name.slice(0, dot) : name}.${options.format}`;
}

/** Waiting screen while a file is being probed. */
const Sniffing: Component<{ path: string }> = (props) => (
  <div class="page sniffing">
    <CatPeek class="sniff-cat" />
    <p class="display sniff-title">Sniffing {baseName(props.path)}…</p>
  </div>
);

export const Editor: Component<{ compact: boolean }> = (props) => (
  <Switch
    fallback={
      <Show when={state.opening} fallback={<EmptyEditor />}>
        {(path) => <Sniffing path={path()} />}
      </Show>
    }
  >
    <Match when={state.choice}>{(paths) => <PictureChoice count={paths().length} />}</Match>
    <Match when={state.sequence} keyed>
      {(draft) => <SequenceEditor draft={draft} compact={props.compact} />}
    </Match>
    <Match when={state.draft} keyed>
      {(draft) => <DraftEditor draft={draft} compact={props.compact} />}
    </Match>
  </Switch>
);

/** Several pictures were dropped: one by one, or one animation? */
const PictureChoice: Component<{ count: number }> = (props) => (
  <div class="page sniffing choice">
    <CatPeek class="sniff-cat" />
    <h1 class="display sniff-title">{props.count} pictures</h1>
    <p class="item-meta">Convert them one by one, or flip through them as one animation?</p>
    <div class="choice-actions">
      <button type="button" class="btn btn-outline btn-lg" onClick={() => convertEach()}>
        <Icon name="convert" size={18} />
        Convert each
      </button>
      <button type="button" class="btn btn-primary btn-lg" onClick={() => void makeAnimation()}>
        <Icon name="sparkle" size={18} />
        Make an animation
      </button>
    </div>
    <button type="button" class="link-btn" onClick={() => closeEditor()}>
      Never mind
    </button>
  </div>
);

const EmptyEditor: Component = () => (
  <div class="page sniffing">
    <CatPeek sleepy class="sniff-cat" />
    <p class="display sniff-title">No file open</p>
    <button type="button" class="btn btn-outline" onClick={() => closeEditor()}>
      Back to Convert
    </button>
  </div>
);

const DraftEditor: Component<{ draft: Draft; compact: boolean }> = (props) => {
  // `draft` is a store proxy, so these stay reactive to option changes.
  const info = () => props.draft.info;
  const options = () => props.draft.options;
  const outKind = () => formatKind(options().format, info().kind);
  const duration = () => info().durationSecs;
  const timed = () => info().kind !== "image" && outKind() !== "image" && (duration() ?? 0) > 0;
  const visual = () => info().kind !== "audio" && outKind() !== "audio";
  const hasPicture = () => info().kind !== "audio";

  const trimStart = () => options().trimStart ?? 0;
  const trimEnd = () => options().trimEnd ?? duration() ?? 0;
  const trimmed = () => options().trimStart !== null || options().trimEnd !== null;

  // Preview -----------------------------------------------------------------

  const [scrub, setScrub] = createSignal(
    info().kind === "video" ? Math.min(1, (duration() ?? 0) * 0.15) : 0,
  );

  // Listening ---------------------------------------------------------------

  /** There is sound to play: an audio file, or a video with a sound track. */
  const canListen = () => timed() && (info().kind === "audio" || info().audioCodec !== null);
  const [loop, setLoop] = createSignal(false);
  /** Measured loudness: `undefined` until asked for, `null` if silent or unknown. */
  const [lufs, setLufs] = createSignal<number | null | undefined>(undefined);
  /** Normalisation as a volume factor, while the output gets it. */
  const level = () => (hasSound() && options().sound.normalize ? normalizeGain(lufs() ?? null) : 1);
  // The option callbacks run only while playing, after everything below exists.
  const player = createPlayer(() => (canListen() ? info().path : null), {
    type: () => playbackType(info().extension, info().audioCodec),
    gain: (t) =>
      hasSound()
        ? soundGain(t, { start: trimStart(), end: trimEnd(), ...fades(), level: level() })
        : 1,
    limit: () => level() > 1,
  });
  const selection = (): PlayRange => ({ start: trimStart(), end: trimEnd(), loop: loop() });
  const playSelection = () => {
    if (player.playing() && player.inRange()) player.pause();
    else player.play(trimStart(), selection);
  };
  /** Which of the sound settings playback lets you hear, if any. */
  const previewNote = () => {
    if (!hasSound() || !player.shapes()) return null;
    const heard: string[] = [];
    if (fades().fadeIn > 0 || fades().fadeOut > 0) heard.push("fades");
    if (options().sound.normalize) heard.push("even loudness");
    return heard.length > 0 ? `You hear the ${heard.join(" and ")} as the copy will.` : null;
  };
  /** Go to `t`: the frame shown and where playback starts or carries on. */
  const scrubTo = (t: number) => {
    setScrub(t);
    player.seek(t);
  };
  player.seek(scrub());
  // A paused video shows the frame it stopped on.
  createEffect(
    on(
      player.playing,
      (now, before) => {
        if (before && !now && info().kind === "video") setScrub(player.time());
      },
      { defer: true },
    ),
  );
  // Opening the next file stops this one.
  createEffect(() => {
    if (state.opening) player.pause();
  });
  // Loudness is measured once, the first time it is heard.
  createEffect(() => {
    if (lufs() !== undefined || !player.playing() || !hasSound() || !options().sound.normalize) {
      return;
    }
    setLufs(null);
    loudness(info().path).then(setLufs, () => {});
  });

  const picturePath = () => (hasPicture() ? info().path : null);
  // While a video plays, the picture follows a few times a second.
  const frameTime = () =>
    player.playing() && !player.loading() ? Math.floor(player.time() * 4) / 4 : scrub();
  const frame = createFrame(picturePath, frameTime, 540, info().hdr);
  const filmstrip = createFilmstrip(
    () => (info().kind === "video" ? info().path : null),
    duration,
    FILMSTRIP_FRAMES,
    90,
    info().hdr,
  );
  const wave = createWaveform(() => (info().kind === "audio" ? info().path : null), "#c93b63");

  const quarterTurn = () => options().rotate % 180 === 90;

  // Rotation is shown as a continuous spin rather than snapping back from 270 to 0.
  // A crop turns and mirrors along with the picture, so it keeps the same pixels.
  const [spin, setSpin] = createSignal(options().rotate as number);
  // When the picture last started to turn or mirror (not reactive).
  let lastTurn = Number.NEGATIVE_INFINITY;
  const rotate = () => {
    lastTurn = performance.now();
    setSpin((s) => s + 90);
    const crop = options().crop;
    updateOptions({
      rotate: ((options().rotate + 90) % 360) as ConvertOptions["rotate"],
      crop: crop && turnCrop(crop),
    });
  };
  const mirror = () => {
    lastTurn = performance.now();
    const crop = options().crop;
    updateOptions({ flip: !options().flip, crop: crop && mirrorCrop(crop) });
  };

  // Crop ----------------------------------------------------------------------

  const picture = () => turnedSize(info(), options().rotate);
  const [cropping, setCropping] = createSignal(false);
  // Size of the preview image on screen, measured because small frames aren't
  // scaled up to fill the preview.
  const [imageSize, setImageSize] = createSignal<Size | null>(null);
  const shown = (): Size | null => {
    const size = imageSize();
    if (!size) return null;
    return quarterTurn() ? { width: size.height, height: size.width } : size;
  };
  let resizeObserver: ResizeObserver | undefined;
  const watchImage = (img: HTMLImageElement) => {
    const measure = () =>
      setImageSize(img.offsetWidth ? { width: img.offsetWidth, height: img.offsetHeight } : null);
    img.addEventListener("load", measure);
    resizeObserver?.disconnect();
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(measure);
      resizeObserver.observe(img);
    }
    queueMicrotask(measure);
  };
  onCleanup(() => resizeObserver?.disconnect());

  const chooseAspect = (aspect: Aspect | null) => {
    const crop = options().crop;
    const size = picture();
    if (aspect) {
      updateOptions({ crop: { aspect: [...aspect], area: null } });
    } else if (crop?.aspect && size) {
      // Free keeps the box where it is, just without a fixed shape.
      updateOptions({
        crop: cropBox(size, crop) ? { aspect: null, area: keptArea(size, crop) } : null,
      });
    }
  };

  // Frames --------------------------------------------------------------------

  const canStep = () => info().kind === "video" && (duration() ?? 0) > 0;
  const canSaveFrame = () => canStep() && state.ffmpeg?.found === true && isAvailable("png");
  const stepBy = (delta: number) => {
    const from = player.playing() ? player.time() : scrub();
    player.pause();
    scrubTo(stepFrame(from, delta, info().fps ?? FALLBACK_FPS, duration() ?? 0));
  };
  const [savingFrame, setSavingFrame] = createSignal(false);
  const saveThisFrame = async () => {
    if (savingFrame()) return;
    setSavingFrame(true);
    try {
      await saveFrame(scrub());
    } finally {
      setSavingFrame(false);
    }
  };

  // Space plays or pauses (Shift+Space the selection), ← → (or , .) step a
  // frame and S saves it, unless the key is meant for a field, button or slider.
  const onKey = (event: KeyboardEvent) => {
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
    if (target?.isContentEditable) return;
    if (event.key === " ") {
      // Space presses a focused button or types in a field. Sliders don't use it.
      const pressable = "input, textarea, select, button, summary, a, [role=switch]";
      if (!canListen() || target?.closest(pressable)) return;
      event.preventDefault();
      if (event.repeat) return;
      if (event.shiftKey) playSelection();
      else player.toggle();
      return;
    }
    if (target?.closest("input, textarea, select, [role=slider]")) return;
    if (!canStep()) return;
    if (event.key === "ArrowLeft" || event.key === ",") stepBy(-1);
    else if (event.key === "ArrowRight" || event.key === ".") stepBy(1);
    else if (event.key.toLowerCase() === "s" && canSaveFrame()) void saveThisFrame();
    else return;
    event.preventDefault();
  };
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  // Command preview ---------------------------------------------------------

  const [command, setCommand] = createSignal("");
  // Open from the start when the file arrives with an edited command.
  const [showCommand, setShowCommand] = createSignal(props.draft.command !== null);
  let commandTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(
    on(
      () => JSON.stringify(options()),
      () => {
        clearTimeout(commandTimer);
        if (!showCommand()) return;
        const snapshot = { ...options() };
        commandTimer = setTimeout(() => {
          previewCommand(info(), snapshot).then(setCommand, (error) => setCommand(`# ${error}`));
        }, 150);
      },
    ),
  );
  createEffect(
    on(showCommand, (open) => {
      if (open)
        previewCommand(info(), { ...options() }).then(setCommand, (e) => setCommand(`# ${e}`));
    }),
  );
  onCleanup(() => clearTimeout(commandTimer));

  // Command edited by hand ----------------------------------------------------

  /** The command was edited: it runs as written and the options don't apply. */
  const custom = () => props.draft.command !== null;
  const [commandError, setCommandError] = createSignal<string | null>(null);
  let commandField: HTMLTextAreaElement | undefined;
  let checkTimer: ReturnType<typeof setTimeout> | undefined;
  let checks = 0;
  // Checked as it's typed, so mistakes show up before converting.
  createEffect(
    on(
      () => [props.draft.command, JSON.stringify(options())] as const,
      ([text]) => {
        clearTimeout(checkTimer);
        const check = ++checks;
        if (text === null) {
          setCommandError(null);
          return;
        }
        const snapshot = { ...options() };
        checkTimer = setTimeout(() => {
          checkCommand(info(), snapshot, text).then(
            () => check === checks && setCommandError(null),
            // As written: messages can start with a program name, like "rm isn't …".
            (error) => check === checks && setCommandError(String(error)),
          );
        }, 250);
      },
    ),
  );
  onCleanup(() => clearTimeout(checkTimer));
  const canEdit = () => command() !== "" && !command().startsWith("#");
  const editCommand = () => {
    setCropping(false);
    setShowCommand(true);
    setCustomCommand(command());
    queueMicrotask(() => commandField?.focus());
  };
  const resetCommand = () => setCustomCommand(null);

  // Trim text fields ----------------------------------------------------------

  const setTrim = (start: number, end: number) => {
    const d = duration() ?? 0;
    updateOptions({
      trimStart: start <= 0.05 ? null : Math.round(start * 100) / 100,
      trimEnd: end >= d - 0.05 ? null : Math.round(end * 100) / 100,
    });
  };

  const commitTime = (which: "start" | "end", input: HTMLInputElement) => {
    const parsed = parseClock(input.value);
    const d = duration() ?? 0;
    if (parsed === null) {
      input.value = formatClock(which === "start" ? trimStart() : trimEnd(), true);
      return;
    }
    if (which === "start") setTrim(Math.min(parsed, trimEnd() - 0.1), trimEnd());
    else setTrim(trimStart(), Math.min(d, Math.max(parsed, trimStart() + 0.1)));
    scrubTo(which === "start" ? trimStart() : trimEnd());
    input.value = formatClock(which === "start" ? trimStart() : trimEnd(), true);
  };

  // Settings ------------------------------------------------------------------

  const sizes = createMemo(() => sizeChoices(info(), options()));
  const rates = createMemo(() => fpsChoices(info()));
  const isJxl = () => options().format === "jxl";
  const animatedImage = () => isAnimatedImage(options().format, info().kind);
  const auto = () => autoLimits(options().format, info().kind);
  const autoSize = () => {
    const limit = auto()?.size;
    return limit !== undefined && (shortSide(info(), options()) ?? 0) > limit ? limit : null;
  };
  // Speeding an animation up packs more source frames into each second.
  const playedFps = () => (info().fps ?? 30) * (animatedImage() ? options().speed : 1);
  const autoFps = () => {
    const limit = auto()?.fps;
    return limit !== undefined && playedFps() > limit + 0.01 ? limit : null;
  };
  const canKeepAudio = () =>
    info().audioCodec !== null && outKind() === "video" && !animatedImage();
  const restoring = () => restoresJpeg(info(), options());
  const repacking = () => repacksJpeg(info(), options());
  const lossless = () => isLossless(options()) || restoring();
  const setJxl = (patch: Partial<ConvertOptions["jxl"]>) =>
    updateOptions({ jxl: { ...options().jxl, ...patch } });
  const hasSound = () => outputHasSound(info(), options());
  /** Length of what gets written, when known. */
  const keptLength = () => (timed() ? trimEnd() - trimStart() : null);
  const fades = () => fadeSpans(options().sound, keptLength());
  const setSound = (patch: Partial<ConvertOptions["sound"]>) =>
    updateOptions({ sound: { ...options().sound, ...patch } });
  const unavailableReason = (format: ConvertOptions["format"]) =>
    format === "jxl"
      ? "Install libjxl (brew install jpeg-xl) or an ffmpeg with libjxl to write JXL"
      : `Your ffmpeg can't write ${label(format)}`;
  const canKeepMetadata = () => keepsMetadata(options().format, info().kind);
  /** What the JPEG repack or restore leaves out, if anything. */
  const leftOut = () =>
    options().metadata === "none"
      ? "its metadata"
      : options().metadata === "noLocation" && info().hasLocation
        ? "its location"
        : null;
  const metadataHint = () => {
    if (!canKeepMetadata()) {
      if (animatedImage()) return "Animations come out without any tags.";
      if (info().kind !== "image") return "Stills taken from a video carry no tags.";
      return `${label(options().format)} files come out without dates, place or camera details.`;
    }
    switch (options().metadata) {
      case "keep":
        return "Dates, place and camera details, where the format allows";
      case "noLocation":
        return "Dates and camera details stay, GPS coordinates go";
      default:
        return "Dates, place, camera details and titles all go";
    }
  };
  // A guess from the options means nothing once the command is edited.
  const estimate = createMemo(() => (custom() ? null : describeEstimate(info(), options())));
  const codec = () => (animatedImage() ? null : codecOf(options()));

  // The quality in numbers: the core says which audio bitrate and CRF the
  // command uses, the estimate adds what a CRF comes to for this picture.
  const [encoding, setEncoding] = createSignal<QualitySummary | null>(null);
  let encodingAsked = 0;
  createEffect(
    on(
      () => [info(), JSON.stringify(options())] as const,
      () => {
        const asked = ++encodingAsked;
        const snapshot = { ...options() };
        Promise.resolve()
          .then(() => qualitySummary(info(), snapshot))
          .then(
            (summary) => asked === encodingAsked && setEncoding(summary ?? null),
            () => asked === encodingAsked && setEncoding(null),
          );
      },
    ),
  );
  // Like the size estimate, these numbers stop applying once the command is edited.
  const qualityNumbers = createMemo(() =>
    custom() ? null : describeQuality(encoding(), estimateVideoBitrate(info(), options())),
  );
  /** The exact bitrate in kbit/s while that mode is on for a video file. */
  const exactBitrate = () => (codec() === null ? null : (options().videoBitrate ?? null));
  /** About what the quality level gives now, as a starting point for an exact bitrate. */
  const suggestedBitrate = () => {
    const estimated = estimateVideoBitrate(info(), { ...options(), videoBitrate: null });
    const bps = estimated ?? info().videoBitrate ?? 4_000_000;
    const scale = 10 ** Math.max(0, Math.floor(Math.log10(bps)) - 1);
    return (Math.round(bps / scale) * scale) / 1000;
  };
  const readable = () => canRead(info());
  const ready = () => {
    const chosen = codec();
    return (
      state.ffmpeg?.found === true &&
      readable() &&
      isAvailable(options().format) &&
      (chosen === null || isCodecAvailable(chosen)) &&
      !(custom() && commandError() !== null)
    );
  };

  const summary = createMemo(() => {
    const o = options();
    const items: {
      icon:
        | "scissors"
        | "image"
        | "rotate"
        | "flip"
        | "crop"
        | "music"
        | "clock"
        | "sparkle"
        | "sliders"
        | "fade"
        | "play"
        | "pin"
        | "terminal";
      text: string;
    }[] = [];
    if (custom()) return [{ icon: "terminal" as const, text: "Custom command" }];
    if (timed() && trimmed()) {
      items.push({ icon: "scissors", text: `Keeps ${formatSpan(trimEnd() - trimStart())}` });
    }
    if (visual()) {
      const size = o.maxSize ?? autoSize();
      if (size !== null) items.push({ icon: "image", text: sizeLabel(size, outKind()) });
      if (o.rotate) items.push({ icon: "rotate", text: `Turned ${o.rotate}°` });
      if (o.flip) items.push({ icon: "flip", text: "Mirrored" });
      const cropped = cropLabel(picture(), o.crop);
      if (cropped) items.push({ icon: "crop", text: cropped });
      const fps = o.fps ?? (outKind() === "video" ? autoFps() : null);
      if (outKind() === "video" && fps !== null) items.push({ icon: "clock", text: `${fps} fps` });
    }
    if (changesPlayback(info(), o)) {
      // The preview is a still, so spell out how the animation will play.
      const parts: string[] = [];
      if (o.speed !== 1) parts.push(speedLabel(o.speed));
      const way = DIRECTIONS.find((d) => d.direction === o.direction);
      if (way && o.direction !== "forward") parts.push(way.label);
      const seconds = outputSeconds(info(), o);
      if (seconds !== null) parts.push(formatSpan(seconds));
      items.push({ icon: "play", text: parts.join(" · ") });
    }
    if (canKeepAudio() && !o.keepAudio) items.push({ icon: "music", text: "No sound" });
    if (hasSound()) {
      if (o.sound.normalize) items.push({ icon: "sliders", text: "Even loudness" });
      const { fadeIn, fadeOut } = fades();
      if (fadeIn > 0) items.push({ icon: "fade", text: `Fades in ${formatSpan(fadeIn)}` });
      if (fadeOut > 0) items.push({ icon: "fade", text: `Fades out ${formatSpan(fadeOut)}` });
    }
    const chosen = codec();
    if (chosen === "h265" && o.hardware && canEncodeHevcInHardware()) {
      items.push({ icon: "sparkle", text: "H.265, hardware" });
    } else if (chosen === "h265" || chosen === "av1") {
      items.push({ icon: "sparkle", text: CODEC_LABELS[chosen] });
    }
    const exact = exactBitrate();
    if (exact !== null) items.push({ icon: "sliders", text: formatBitrate(exact * 1000) });
    if (isJxl() && info().kind === "video") items.push({ icon: "sparkle", text: "Animated" });
    if (isJxl() && o.jxl.lossless) items.push({ icon: "sparkle", text: "Lossless" });
    if (info().hasLocation && !keepsLocation(info(), o)) {
      items.push({ icon: "pin", text: "No location" });
    }
    return items;
  });

  // Saving as a recipe ------------------------------------------------------

  const [naming, setNaming] = createSignal(false);
  let nameInput: HTMLInputElement | undefined;
  const submitRecipe = (event: SubmitEvent) => {
    event.preventDefault();
    void saveRecipe(nameInput?.value ?? "");
    setNaming(false);
  };

  const convertLabel = () => (state.inbox.length > 0 ? "Convert & next" : "Start converting");

  const convertButton = (cls: string) => (
    <button
      type="button"
      class={`btn btn-primary ${cls}`}
      disabled={!ready()}
      onClick={() => void convertDraft()}
    >
      <Icon name="paw" size={18} />
      {convertLabel()}
    </button>
  );

  return (
    <div class="page editor">
      <header class="file-header">
        <button
          type="button"
          class="icon-btn square"
          aria-label="Back"
          onClick={() => closeEditor()}
        >
          <Icon name="chevron-left" size={22} />
        </button>
        <div class="file-heading">
          <h1 class="display file-title" title={info().fileName}>
            {info().fileName}
          </h1>
          <p class="item-meta">{describe(info())}</p>
        </div>
        <Show when={!props.compact}>{convertButton("btn-lg")}</Show>
      </header>

      <Show when={state.inbox.length > 0}>
        <div class="inbox">
          <span class="inbox-label">Up next</span>
          <ul class="inbox-list">
            <For each={state.inbox.slice(0, 3)}>
              {(path) => <li class="inbox-chip">{baseName(path)}</li>}
            </For>
            <Show when={state.inbox.length > 3}>
              <li class="inbox-chip more">+{state.inbox.length - 3}</li>
            </Show>
          </ul>
          <div class="inbox-actions">
            <button
              type="button"
              class="link-btn"
              disabled={!ready()}
              title="Queue every waiting file with these settings. The trim and where the crop sits stay with this file."
              onClick={() => void convertAll()}
            >
              <Icon name="paw" size={14} /> Convert all {state.inbox.length + 1} like this
            </button>
            <button type="button" class="link-btn" onClick={() => skipDraft()}>
              <Icon name="skip" size={14} /> Skip this one
            </button>
          </div>
        </div>
      </Show>

      <div class="file-grid">
        <div class="stack">
          <div
            class="preview"
            data-kind={info().kind}
            classList={{
              "out-audio": outKind() === "audio" && info().kind !== "audio",
              cropping: cropping() && visual(),
            }}
          >
            <Show
              when={hasPicture()}
              fallback={
                <div class="audio-art">
                  <CatPeek class="audio-cat" />
                  <span class="note n1">♪</span>
                  <span class="note n2">♫</span>
                  <span class="note n3">♪</span>
                </div>
              }
            >
              <Show when={frame()} fallback={<div class="preview-skeleton" />}>
                {(src) => (
                  <img
                    ref={watchImage}
                    class="preview-img"
                    classList={{ quarter: quarterTurn() }}
                    src={src()}
                    alt={`Preview of ${info().fileName}`}
                    style={{
                      transform: `rotate(${spin()}deg) scaleX(${options().flip ? -1 : 1})`,
                    }}
                    draggable={false}
                  />
                )}
              </Show>
            </Show>
            <Show when={visual() && picture() && shown() && (cropping() || options().crop)}>
              {/* Recreated after a turn or mirror so it fades in once the picture settles. */}
              <Show when={`${options().rotate}-${options().flip}`} keyed>
                {(_turn) => (
                  <CropOverlay
                    picture={picture() as Size}
                    shown={shown() as Size}
                    crop={options().crop}
                    editing={cropping()}
                    settle={performance.now() - lastTurn < 600}
                    onChange={(crop) => updateOptions({ crop })}
                    onDone={() => setCropping(false)}
                  />
                )}
              </Show>
            </Show>
            <Show when={visual() && cropping()}>
              <fieldset class="crop-bar" aria-label="Crop shape">
                <For each={ASPECTS}>
                  {(preset) => (
                    <button
                      type="button"
                      class="crop-chip"
                      aria-pressed={sameAspect(options().crop?.aspect ?? null, preset.aspect)}
                      onClick={() => chooseAspect(preset.aspect)}
                    >
                      <span
                        class="aspect-glyph"
                        classList={{ free: !preset.aspect }}
                        style={
                          preset.aspect
                            ? { "aspect-ratio": `${preset.aspect[0]} / ${preset.aspect[1]}` }
                            : undefined
                        }
                      />
                      {preset.label}
                    </button>
                  )}
                </For>
                <span class="crop-bar-gap" />
                <button
                  type="button"
                  class="crop-chip plain"
                  disabled={!options().crop}
                  onClick={() => updateOptions({ crop: null })}
                >
                  Reset
                </button>
                <button
                  type="button"
                  class="crop-chip done"
                  aria-label="Done cropping"
                  onClick={() => setCropping(false)}
                >
                  <Icon name="check" size={16} />
                </button>
              </fieldset>
            </Show>
            <Show when={info().kind === "video"}>
              <div class="frame-bar">
                <div class="frame-pill">
                  <Show when={canStep()}>
                    <button
                      type="button"
                      class="frame-step"
                      aria-label="Previous frame"
                      title="Previous frame (←)"
                      onClick={() => stepBy(-1)}
                    >
                      <Icon name="chevron-left" size={16} />
                    </button>
                  </Show>
                  <span class="frame-time mono">{formatClock(scrub(), true)}</span>
                  <Show when={canStep()}>
                    <button
                      type="button"
                      class="frame-step"
                      aria-label="Next frame"
                      title="Next frame (→)"
                      onClick={() => stepBy(1)}
                    >
                      <Icon name="chevron-right" size={16} />
                    </button>
                  </Show>
                </div>
                <Show when={canSaveFrame()}>
                  <button
                    type="button"
                    class="frame-pill frame-save"
                    title="Save this frame as a PNG next to the video (S)"
                    aria-busy={savingFrame()}
                    disabled={savingFrame()}
                    onClick={() => void saveThisFrame()}
                  >
                    <Icon name="camera" size={16} />
                    <span class="frame-save-label">Save this frame</span>
                  </button>
                </Show>
              </div>
            </Show>
            <Show when={outKind() === "audio" && info().kind === "video"}>
              <span class="preview-badge right">
                <Icon name="music" size={14} /> Sound only
              </span>
            </Show>
            <Show when={visual() && !custom()}>
              <div class="preview-tools">
                <button
                  type="button"
                  class="tool-btn"
                  aria-label="Rotate 90°"
                  title="Rotate 90°"
                  onClick={rotate}
                >
                  <Icon name="rotate" size={20} />
                </button>
                <button
                  type="button"
                  class="tool-btn"
                  aria-label="Mirror"
                  title="Mirror"
                  aria-pressed={options().flip}
                  onClick={mirror}
                >
                  <Icon name="flip" size={20} />
                </button>
                <Show when={picture()}>
                  <button
                    type="button"
                    class="tool-btn"
                    aria-label="Crop"
                    title="Crop"
                    aria-pressed={cropping()}
                    onClick={() => setCropping(!cropping())}
                  >
                    <Icon name="crop" size={20} />
                  </button>
                </Show>
              </div>
            </Show>
          </div>

          <Show when={timed()}>
            <section class="card trim">
              <div class="row-between">
                <h2 class="label-strong">
                  <Icon name="scissors" size={16} /> Trim
                </h2>
                <span class="item-meta">
                  <Show when={trimmed()} fallback="Drag the handles to cut">
                    Keeping {formatClock(trimStart(), true)} – {formatClock(trimEnd(), true)} ·{" "}
                    {formatSpan(trimEnd() - trimStart())}
                  </Show>
                </span>
              </div>
              <TrimBar
                duration={duration() ?? 0}
                start={trimStart()}
                end={trimEnd()}
                playhead={canListen() ? player.time() : info().kind === "video" ? scrub() : null}
                playing={player.playing()}
                fades={hasSound() ? fades() : undefined}
                // An edited command carries its own trim; listening and seeking still work.
                onChange={(start, end) => !custom() && setTrim(start, end)}
                // Moving a handle while playing leaves the sound where it is.
                onScrub={(t) => {
                  if (!player.playing()) scrubTo(t);
                }}
                onSeek={scrubTo}
              >
                <Show
                  when={info().kind === "video"}
                  fallback={
                    <Show when={wave()}>
                      {(src) => <img class="trim-wave" src={src()} alt="" draggable={false} />}
                    </Show>
                  }
                >
                  <div class="filmstrip">
                    <Index each={filmstrip()}>
                      {(src) => (
                        <span class="film-frame">
                          <Show when={src()}>
                            {(s) => <img src={s()} alt="" draggable={false} />}
                          </Show>
                        </span>
                      )}
                    </Index>
                  </div>
                </Show>
              </TrimBar>
              <Show when={canListen()}>
                <fieldset class="player-bar" aria-label="Playback">
                  <button
                    type="button"
                    class="play-btn"
                    aria-label={player.playing() ? "Pause" : "Play"}
                    aria-keyshortcuts="Space"
                    title={player.playing() ? "Pause (Space)" : "Play (Space)"}
                    aria-busy={player.loading()}
                    onClick={() => player.toggle()}
                  >
                    <Icon name={player.playing() ? "pause" : "play"} size={20} />
                  </button>
                  <button
                    type="button"
                    class="player-chip"
                    aria-pressed={player.playing() && player.inRange()}
                    aria-keyshortcuts="Shift+Space"
                    title="Play just the part you keep (Shift+Space)"
                    onClick={playSelection}
                  >
                    <Icon
                      name={player.playing() && player.inRange() ? "pause" : "scissors"}
                      size={15}
                    />
                    Play selection
                  </button>
                  <button
                    type="button"
                    class="player-chip"
                    aria-pressed={loop()}
                    title="Start the selection over when it ends"
                    onClick={() => setLoop(!loop())}
                  >
                    <Icon name="repeat" size={15} />
                    Loop
                  </button>
                  <span class="player-time mono">
                    {formatClock(player.time(), true)}
                    <span class="player-of">
                      {" "}
                      / {formatClock(player.duration() ?? duration() ?? 0, true)}
                    </span>
                  </span>
                </fieldset>
                <Show
                  when={player.error()}
                  fallback={
                    <Show when={previewNote()}>
                      {(note) => <p class="item-meta player-note">{note()}</p>}
                    </Show>
                  }
                >
                  {(message) => (
                    <p class="player-error" role="alert">
                      <Icon name="alert" size={16} /> {message()}
                    </p>
                  )}
                </Show>
              </Show>
              <div
                class="trim-fields"
                classList={{ locked: custom() }}
                inert={custom()}
                aria-disabled={custom() || undefined}
              >
                <label class="field">
                  <span class="field-label">Start</span>
                  <input
                    class="input mono"
                    value={formatClock(trimStart(), true)}
                    spellcheck={false}
                    onChange={(e) => commitTime("start", e.currentTarget)}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  />
                </label>
                <label class="field">
                  <span class="field-label">End</span>
                  <input
                    class="input mono"
                    value={formatClock(trimEnd(), true)}
                    spellcheck={false}
                    onChange={(e) => commitTime("end", e.currentTarget)}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  />
                </label>
                <button
                  type="button"
                  class="btn btn-outline reset-trim"
                  disabled={!trimmed()}
                  onClick={() => updateOptions({ trimStart: null, trimEnd: null })}
                >
                  Keep it all
                </button>
              </div>
            </section>
          </Show>
        </div>

        <section class="card settings">
          <Show when={custom()}>
            <div class="lossless-note custom-note" role="status">
              <Icon name="terminal" size={16} />
              <span>Your edited command runs as written, so these settings no longer apply.</span>
              <button type="button" class="link-btn" onClick={resetCommand}>
                Reset to generated
              </button>
            </div>
          </Show>

          <fieldset class="option-controls" disabled={custom()}>
            <fieldset class="field">
              <legend class="field-label">Turn it into</legend>
              <div class="format-groups">
                <For each={groupsFor(info().kind)}>
                  {(group) => (
                    <div class="format-group" data-tone={group.tone}>
                      <span class="format-group-label">{group.label}</span>
                      <div class="format-chips">
                        <For each={group.formats}>
                          {(format) => (
                            <button
                              type="button"
                              class="format-chip"
                              aria-pressed={options().format === format}
                              disabled={!isAvailable(format)}
                              title={isAvailable(format) ? undefined : unavailableReason(format)}
                              onClick={() => updateOptions({ format })}
                            >
                              {label(format)}
                            </button>
                          )}
                        </For>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </fieldset>

            <Show when={!readable()}>
              <p class="lossless-note warning-note" role="alert">
                <Icon name="alert" size={16} />
                This file is AV1, which your ffmpeg can't read. Install one built with dav1d.
              </p>
            </Show>

            <Show when={!animatedImage()}>
              <CodecField
                format={options().format}
                codec={options().videoCodec ?? null}
                available={isCodecAvailable}
                onChange={(videoCodec) => updateOptions({ videoCodec })}
              />
            </Show>

            <Show when={codec() === "h265" && canEncodeHevcInHardware()}>
              <div class="toggle-row">
                <div>
                  <span class="toggle-title" id="hardware-encoding">
                    Fast (hardware)
                  </span>
                  <span class="item-meta">
                    Encodes on the Mac's media engine: several times faster, somewhat bigger files
                  </span>
                </div>
                <button
                  type="button"
                  class="switch"
                  role="switch"
                  aria-checked={options().hardware ?? false}
                  aria-labelledby="hardware-encoding"
                  onClick={() => updateOptions({ hardware: !options().hardware })}
                />
              </div>
            </Show>

            <Show
              when={!lossless()}
              fallback={
                <p class="lossless-note">
                  <Icon name="sparkle" size={16} />
                  {restoring()
                    ? `Restores the original JPEG, bit for bit${leftOut() ? ` apart from ${leftOut()}` : ""}.`
                    : repacking()
                      ? `Repacks the JPEG without re-encoding: smaller, and the original can be restored exactly${leftOut() ? `, minus ${leftOut()}` : ""}.`
                      : "Lossless: every pixel and sample stays put."}
                  <Show when={!restoring() && !repacking() && qualityNumbers()}>
                    {(text) => <span class="lossless-numbers">{text()}</span>}
                  </Show>
                </p>
              }
            >
              <QualityField
                quality={options().quality}
                numbers={qualityNumbers()}
                describedBy={estimate() ? "size-estimate" : undefined}
                onChange={(quality) => updateOptions({ quality })}
                bitrate={exactBitrate()}
                onBitrate={
                  codec() === null ? undefined : (videoBitrate) => updateOptions({ videoBitrate })
                }
                suggestedBitrate={suggestedBitrate}
              />
            </Show>

            <Show when={estimate()}>
              {(text) => (
                <p
                  id="size-estimate"
                  class="estimate"
                  title="A rough guess from the original and these settings. The real size depends on what's in the file."
                >
                  {text()}
                </p>
              )}
            </Show>

            <Show when={visual() || (outKind() === "video" && info().kind === "video")}>
              <div class="two-col">
                <Show when={visual() && info().width !== null}>
                  <label class="field">
                    <span class="field-label">Size</span>
                    <select
                      class="select"
                      onChange={(e) => {
                        const v = e.currentTarget.value;
                        updateOptions({ maxSize: v === "" ? null : Number(v) });
                      }}
                    >
                      <option value="" selected={options().maxSize === null}>
                        {autoSize() !== null
                          ? `Auto (${sizeLabel(autoSize() ?? 0, outKind())})`
                          : `Original (${sizeLabel(shortSide(info(), options()) ?? 0, info().kind)})`}
                      </option>
                      <For each={sizes()}>
                        {(size) => (
                          <option value={size} selected={options().maxSize === size}>
                            {sizeLabel(size, outKind())}
                          </option>
                        )}
                      </For>
                    </select>
                  </label>
                </Show>
                <Show when={outKind() === "video" && info().kind === "video"}>
                  <label class="field">
                    <span class="field-label">Frame rate</span>
                    <select
                      class="select"
                      onChange={(e) => {
                        const v = e.currentTarget.value;
                        updateOptions({ fps: v === "" ? null : Number(v) });
                      }}
                    >
                      <option value="" selected={options().fps === null}>
                        {autoFps() !== null
                          ? `Auto (${autoFps()})`
                          : `Keep${info().fps ? ` (${Math.round(info().fps ?? 0)})` : ""}`}
                      </option>
                      <For each={rates()}>
                        {(fps) => (
                          <option value={fps} selected={options().fps === fps}>
                            {fps} fps
                          </option>
                        )}
                      </For>
                    </select>
                  </label>
                </Show>
              </div>
            </Show>

            <Show when={animatedImage()}>
              <fieldset class="field">
                <legend class="field-label">Playback</legend>
                <div class="format-groups playback" data-tone="pink">
                  <fieldset class="format-group" aria-labelledby="playback-speed">
                    <span class="format-group-label" id="playback-speed">
                      Speed
                    </span>
                    <div class="format-chips">
                      <For each={SPEEDS}>
                        {(speed) => (
                          <button
                            type="button"
                            class="format-chip"
                            aria-pressed={options().speed === speed}
                            onClick={() => updateOptions({ speed })}
                          >
                            {speedLabel(speed)}
                          </button>
                        )}
                      </For>
                    </div>
                  </fieldset>
                  <fieldset class="format-group" aria-labelledby="playback-direction">
                    <span class="format-group-label" id="playback-direction">
                      Direction
                    </span>
                    <div class="format-chips">
                      <For each={DIRECTIONS}>
                        {(way) => (
                          <button
                            type="button"
                            class="format-chip"
                            aria-pressed={options().direction === way.direction}
                            onClick={() => updateOptions({ direction: way.direction })}
                          >
                            {way.label}
                          </button>
                        )}
                      </For>
                    </div>
                  </fieldset>
                </div>
              </fieldset>
            </Show>

            <Show when={canKeepAudio()}>
              <div class="toggle-row">
                <div>
                  <span class="toggle-title" id="keep-audio">
                    Keep sound
                  </span>
                  <span class="item-meta">{info().audioCodec?.toUpperCase()} track</span>
                </div>
                <button
                  type="button"
                  class="switch"
                  role="switch"
                  aria-checked={options().keepAudio}
                  aria-labelledby="keep-audio"
                  onClick={() => updateOptions({ keepAudio: !options().keepAudio })}
                />
              </div>
            </Show>

            <Show when={hasSound()}>
              <section class="sound-settings" aria-labelledby="sound-heading">
                <h2 class="label-strong sound-heading" id="sound-heading">
                  <Icon name="music" size={16} /> Sound
                </h2>
                <div class="toggle-row">
                  <div>
                    <span class="toggle-title" id="sound-normalize">
                      Even out loudness
                    </span>
                    <span class="item-meta">A steady, comfortable volume, like streaming apps</span>
                  </div>
                  <button
                    type="button"
                    class="switch"
                    role="switch"
                    aria-checked={options().sound.normalize}
                    aria-labelledby="sound-normalize"
                    onClick={() => setSound({ normalize: !options().sound.normalize })}
                  />
                </div>
                <div class="two-col">
                  <fieldset class="field">
                    <legend class="field-label">Fade in</legend>
                    <div class="format-chips">
                      <For each={FADE_CHOICES}>
                        {(secs) => (
                          <button
                            type="button"
                            class="format-chip"
                            aria-pressed={options().sound.fadeIn === secs}
                            onClick={() => setSound({ fadeIn: secs })}
                          >
                            {secs === 0 ? "Off" : `${secs} s`}
                          </button>
                        )}
                      </For>
                    </div>
                  </fieldset>
                  <Show when={keptLength() !== null}>
                    <fieldset class="field">
                      <legend class="field-label">Fade out</legend>
                      <div class="format-chips">
                        <For each={FADE_CHOICES}>
                          {(secs) => (
                            <button
                              type="button"
                              class="format-chip"
                              aria-pressed={options().sound.fadeOut === secs}
                              onClick={() => setSound({ fadeOut: secs })}
                            >
                              {secs === 0 ? "Off" : `${secs} s`}
                            </button>
                          )}
                        </For>
                      </div>
                    </fieldset>
                  </Show>
                </div>
                <Show when={outKind() === "video" && (fades().fadeIn > 0 || fades().fadeOut > 0)}>
                  <span class="item-meta">The picture fades from and to black with the sound.</span>
                </Show>
              </section>
            </Show>

            <fieldset class="field metadata-field">
              <legend class="field-label">Metadata</legend>
              <Show when={locationLine(info())}>
                {(line) => (
                  <p class="location-line">
                    <span aria-hidden="true">📍</span>
                    <span>
                      {line()}.{" "}
                      <span class="location-fate">
                        {keepsLocation(info(), options())
                          ? "The copy will too."
                          : "The copy won't."}
                      </span>
                    </span>
                  </p>
                )}
              </Show>
              <Show when={canKeepMetadata()}>
                <div class="format-chips">
                  <For each={METADATA_CHOICES}>
                    {(choice) => (
                      <button
                        type="button"
                        class="format-chip"
                        aria-pressed={options().metadata === choice.mode}
                        onClick={() => updateOptions({ metadata: choice.mode })}
                      >
                        {choice.label}
                      </button>
                    )}
                  </For>
                </div>
              </Show>
              <span class="item-meta">{metadataHint()}</span>
            </fieldset>

            <Show when={isJxl()}>
              <div class="jxl-settings">
                <div class="toggle-row">
                  <div>
                    <span class="toggle-title" id="jxl-lossless">
                      Lossless
                    </span>
                    <span class="item-meta">
                      {info().extension === "jpg" || info().extension === "jpeg"
                        ? "Repack the JPEG exactly, about 20% smaller"
                        : "Keep every pixel exactly"}
                    </span>
                  </div>
                  <button
                    type="button"
                    class="switch"
                    role="switch"
                    aria-checked={options().jxl.lossless}
                    aria-labelledby="jxl-lossless"
                    onClick={() => setJxl({ lossless: !options().jxl.lossless })}
                  />
                </div>
                <fieldset class="field">
                  <legend class="field-label">Effort</legend>
                  <div class="format-chips">
                    <For each={JXL_EFFORTS}>
                      {(preset) => (
                        <button
                          type="button"
                          class="format-chip"
                          aria-pressed={options().jxl.effort === preset.effort}
                          onClick={() => setJxl({ effort: preset.effort })}
                        >
                          {preset.label}
                        </button>
                      )}
                    </For>
                  </div>
                  <span class="item-meta">More effort: smaller file, slower to make.</span>
                </fieldset>
                <div class="toggle-row">
                  <div>
                    <span class="toggle-title" id="jxl-progressive">
                      Progressive
                    </span>
                    <span class="item-meta">Shows a sharp preview while it loads</span>
                  </div>
                  <button
                    type="button"
                    class="switch"
                    role="switch"
                    aria-checked={options().jxl.progressive}
                    aria-labelledby="jxl-progressive"
                    onClick={() => setJxl({ progressive: !options().jxl.progressive })}
                  />
                </div>
              </div>
            </Show>
          </fieldset>

          <div class="outcome">
            <div class="outcome-name">
              <Icon name="arrow-right" size={18} />
              <span class="item-name">{outputName(info(), options())}</span>
            </div>
            <Show
              when={summary().length > 0}
              fallback={<p class="item-meta">Same picture and length, fresh format.</p>}
            >
              <ul class="outcome-chips" aria-label="Changes">
                <For each={summary()}>
                  {(item) => (
                    <li class="outcome-chip">
                      <Icon name={item.icon} size={14} />
                      {item.text}
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            <p class="item-meta">Saved next to the original.</p>
          </div>

          <Show
            when={naming()}
            fallback={
              <button type="button" class="link-btn save-recipe" onClick={() => setNaming(true)}>
                <Icon name="star" size={14} /> Save these settings as a recipe
              </button>
            }
          >
            <form class="recipe-form" onSubmit={submitRecipe}>
              <input
                ref={(el) => {
                  nameInput = el;
                  queueMicrotask(() => {
                    el.focus();
                    el.select();
                  });
                }}
                class="input"
                aria-label="Recipe name"
                value={`My ${label(options().format)}`}
                maxLength={40}
                spellcheck={false}
                onKeyDown={(e) => e.key === "Escape" && setNaming(false)}
              />
              <button type="submit" class="btn btn-primary">
                Save
              </button>
              <button type="button" class="btn btn-outline" onClick={() => setNaming(false)}>
                Cancel
              </button>
            </form>
            <p class="item-meta">
              {custom()
                ? "Keeps the edited command as written, trims and turns included; only the file names change."
                : "Keeps the format, settings and crop shape; trims and turns stay with each file."}
            </p>
          </Show>

          <details
            class="command"
            classList={{ custom: custom() }}
            open={showCommand()}
            onToggle={(e) => setShowCommand(e.currentTarget.open)}
          >
            <summary>
              <Icon name="terminal" size={16} /> {custom() ? "Custom command" : "Show command"}
            </summary>
            <Show
              when={custom()}
              fallback={
                <>
                  <pre class="command-text">
                    <code>{command() || "…"}</code>
                  </pre>
                  <div class="command-actions">
                    <button
                      type="button"
                      class="link-btn"
                      disabled={!canEdit()}
                      title="Change the arguments by hand. The settings above stop applying until you reset."
                      onClick={editCommand}
                    >
                      <Icon name="sliders" size={14} /> Edit
                    </button>
                  </div>
                </>
              }
            >
              <textarea
                ref={commandField}
                class="input command-text command-input"
                aria-label="Command"
                aria-invalid={commandError() !== null}
                aria-describedby="command-hint"
                aria-errormessage={commandError() ? "command-error" : undefined}
                value={props.draft.command ?? ""}
                rows={Math.min(12, Math.max(4, (props.draft.command ?? "").split("\n").length + 1))}
                spellcheck={false}
                autocomplete="off"
                autocapitalize="off"
                onInput={(e) => setCustomCommand(e.currentTarget.value)}
              />
              <Show when={commandError()}>
                {(message) => (
                  <p id="command-error" class="command-error" role="alert">
                    <Icon name="alert" size={14} />
                    {message()}
                  </p>
                )}
              </Show>
              <p id="command-hint" class="item-meta">
                Runs ffmpeg, cjxl and djxl directly, never through a shell. The output stays{" "}
                {outputName(info(), options())} next to the original.
              </p>
              <div class="command-actions">
                <button type="button" class="link-btn" onClick={resetCommand}>
                  <Icon name="rotate" size={14} /> Reset to generated
                </button>
              </div>
            </Show>
          </details>
        </section>
      </div>

      <Show when={props.compact}>
        <div class="sticky-cta">{convertButton("btn-block")}</div>
      </Show>
    </div>
  );
};
