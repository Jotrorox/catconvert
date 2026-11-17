import {
  type Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  Index,
  on,
  onCleanup,
  Show,
} from "solid-js";
import { CatPeek } from "../components/Cat";
import { Icon } from "../components/Icon";
import { TrimBar } from "../components/TrimBar";
import { type ConvertOptions, type MediaInfo, previewCommand } from "../lib/api";
import { describeEstimate } from "../lib/estimate";
import { baseName, formatBytes, formatClock, formatSpan, parseClock } from "../lib/format";
import {
  autoLimits,
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
  QUALITY_STEPS,
  qualityStep,
  repacksJpeg,
  restoresJpeg,
  shortSide,
  sizeChoices,
  sizeLabel,
} from "../lib/formats";
import { createFilmstrip, createFrame, createWaveform } from "../lib/preview";
import {
  closeEditor,
  convertAll,
  convertDraft,
  type Draft,
  isAvailable,
  saveRecipe,
  skipDraft,
  state,
  updateOptions,
} from "../lib/store";

const FILMSTRIP_FRAMES = 10;

function describe(info: MediaInfo): string {
  const parts: string[] = [];
  if (info.width && info.height) parts.push(`${info.width}×${info.height}`);
  if (info.durationSecs) parts.push(formatClock(info.durationSecs));
  if (info.videoCodec?.startsWith("jpegxl")) parts.push("JPEG XL");
  else if (info.videoCodec && info.kind === "video") parts.push(info.videoCodec.toUpperCase());
  else if (info.audioCodec && info.kind === "audio") parts.push(info.audioCodec.toUpperCase());
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
  <Show
    when={state.draft}
    keyed
    fallback={
      <Show when={state.opening} fallback={<EmptyEditor />}>
        {(path) => <Sniffing path={path()} />}
      </Show>
    }
  >
    {(draft) => <DraftEditor draft={draft} compact={props.compact} />}
  </Show>
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
  const picturePath = () => (hasPicture() ? info().path : null);
  const frame = createFrame(picturePath, scrub, 540);
  const filmstrip = createFilmstrip(
    () => (info().kind === "video" ? info().path : null),
    duration,
    FILMSTRIP_FRAMES,
    90,
  );
  const wave = createWaveform(() => (info().kind === "audio" ? info().path : null), "#c93b63");

  const quarterTurn = () => options().rotate % 180 === 90;

  // Rotation is shown as a continuous spin rather than snapping back from 270 to 0.
  const [spin, setSpin] = createSignal(options().rotate as number);
  const rotate = () => {
    setSpin((s) => s + 90);
    updateOptions({ rotate: ((options().rotate + 90) % 360) as ConvertOptions["rotate"] });
  };

  // Command preview ---------------------------------------------------------

  const [command, setCommand] = createSignal("");
  const [showCommand, setShowCommand] = createSignal(false);
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
    setScrub(which === "start" ? trimStart() : trimEnd());
    input.value = formatClock(which === "start" ? trimStart() : trimEnd(), true);
  };

  // Settings ------------------------------------------------------------------

  const step = () => qualityStep(options().quality);
  const sizes = createMemo(() => sizeChoices(info()));
  const rates = createMemo(() => fpsChoices(info()));
  const isJxl = () => options().format === "jxl";
  const animatedImage = () => isAnimatedImage(options().format, info().kind);
  const auto = () => autoLimits(options().format, info().kind);
  const autoSize = () => {
    const limit = auto()?.size;
    return limit !== undefined && (shortSide(info()) ?? 0) > limit ? limit : null;
  };
  const autoFps = () => {
    const limit = auto()?.fps;
    return limit !== undefined && (info().fps ?? 30) > limit + 0.01 ? limit : null;
  };
  const canKeepAudio = () =>
    info().audioCodec !== null && outKind() === "video" && !animatedImage();
  const restoring = () => restoresJpeg(info(), options());
  const repacking = () => repacksJpeg(info(), options());
  const lossless = () => isLossless(options()) || restoring();
  const setJxl = (patch: Partial<ConvertOptions["jxl"]>) =>
    updateOptions({ jxl: { ...options().jxl, ...patch } });
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
  const estimate = createMemo(() => describeEstimate(info(), options()));
  const ready = () => state.ffmpeg?.found === true && isAvailable(options().format);

  const summary = createMemo(() => {
    const o = options();
    const items: {
      icon: "scissors" | "image" | "rotate" | "flip" | "music" | "clock" | "sparkle" | "pin";
      text: string;
    }[] = [];
    if (timed() && trimmed()) {
      items.push({ icon: "scissors", text: `Keeps ${formatSpan(trimEnd() - trimStart())}` });
    }
    if (visual()) {
      const size = o.maxSize ?? autoSize();
      if (size !== null) items.push({ icon: "image", text: sizeLabel(size, outKind()) });
      if (o.rotate) items.push({ icon: "rotate", text: `Turned ${o.rotate}°` });
      if (o.flip) items.push({ icon: "flip", text: "Mirrored" });
      const fps = o.fps ?? (outKind() === "video" ? autoFps() : null);
      if (outKind() === "video" && fps !== null) items.push({ icon: "clock", text: `${fps} fps` });
    }
    if (canKeepAudio() && !o.keepAudio) items.push({ icon: "music", text: "No sound" });
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
    saveRecipe(nameInput?.value ?? "");
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
              title="Queue every waiting file with these settings. The trim stays with this file."
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
            classList={{ "out-audio": outKind() === "audio" && info().kind !== "audio" }}
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
            <Show when={info().kind === "video"}>
              <span class="preview-badge mono">{formatClock(scrub(), true)}</span>
            </Show>
            <Show when={outKind() === "audio" && info().kind === "video"}>
              <span class="preview-badge right">
                <Icon name="music" size={14} /> Sound only
              </span>
            </Show>
            <Show when={visual()}>
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
                  onClick={() => updateOptions({ flip: !options().flip })}
                >
                  <Icon name="flip" size={20} />
                </button>
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
                playhead={info().kind === "video" ? scrub() : null}
                onChange={setTrim}
                onScrub={setScrub}
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
              <div class="trim-fields">
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
              </p>
            }
          >
            <div class="field">
              <label class="field-label" for="quality">
                Quality
              </label>
              <input
                id="quality"
                class="range"
                type="range"
                min="0"
                max="100"
                aria-describedby={estimate() ? "size-estimate" : undefined}
                value={options().quality}
                style={{ "--pct": `${options().quality}%` }}
                onInput={(event) => updateOptions({ quality: event.currentTarget.valueAsNumber })}
              />
              <div class="range-scale">
                <For each={QUALITY_STEPS}>
                  {(name) => <span classList={{ active: step() === name }}>{name}</span>}
                </For>
              </div>
            </div>
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
                        : `Original (${sizeLabel(shortSide(info()) ?? 0, info().kind)})`}
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

          <fieldset class="field metadata-field">
            <legend class="field-label">Metadata</legend>
            <Show when={locationLine(info())}>
              {(line) => (
                <p class="location-line">
                  <span aria-hidden="true">📍</span>
                  <span>
                    {line()}.{" "}
                    <span class="location-fate">
                      {keepsLocation(info(), options()) ? "The copy will too." : "The copy won't."}
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
              Keeps the format and settings; trims and turns stay with each file.
            </p>
          </Show>

          <details
            class="command"
            open={showCommand()}
            onToggle={(e) => setShowCommand(e.currentTarget.open)}
          >
            <summary>
              <Icon name="terminal" size={16} /> Show command
            </summary>
            <pre class="command-text">
              <code>{command() || "…"}</code>
            </pre>
          </details>
        </section>
      </div>

      <Show when={props.compact}>
        <div class="sticky-cta">{convertButton("btn-block")}</div>
      </Show>
    </div>
  );
};
