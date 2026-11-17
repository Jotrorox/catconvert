import {
  type Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  Show,
} from "solid-js";
import { CodecField } from "../components/CodecField";
import { Icon } from "../components/Icon";
import { OutputField } from "../components/OutputField";
import { QualityField } from "../components/QualityField";
import {
  type ConvertOptions,
  type Fit,
  type OutputPreview,
  previewSequenceCommand,
  type QualitySummary,
  qualitySummary,
  type SequenceOptions,
  sequenceOutputPreview,
} from "../lib/api";
import {
  describeQuality,
  describeSequenceEstimate,
  estimateSequenceVideoBitrate,
} from "../lib/estimate";
import { formatBytes, formatSpan } from "../lib/format";
import {
  codecOf,
  DIRECTIONS,
  FRAME_TIMES,
  isLossless,
  JXL_EFFORTS,
  label,
  playbackOrder,
  SEQUENCE_FORMATS,
  sequenceAutoSize,
  sequenceFrame,
  sequenceName,
  shortSide,
  sizeChoices,
} from "../lib/formats";
import { createMediaQuery } from "../lib/media";
import { targetOf } from "../lib/output";
import { createThumbnails } from "../lib/preview";
import {
  closeEditor,
  convertSequence,
  isAnimated,
  isCodecAvailable,
  movePicture,
  removePicture,
  type SequenceDraft,
  sortPictures,
  state,
  updateSequence,
  updateSequenceOptions,
} from "../lib/store";

const FITS: { value: Fit; label: string; hint: string }[] = [
  { value: "pad", label: "Black bars", hint: "Shows every picture whole" },
  { value: "crop", label: "Crop to fill", hint: "Fills the frame, trims the edges" },
];

const secondsLabel = (secs: number) => `${secs} s`;

/** Height of the frames grabbed for the preview and filmstrip. */
const THUMB_HEIGHT = 360;

export const SequenceEditor: Component<{ draft: SequenceDraft; compact: boolean }> = (props) => {
  // `draft` is a store proxy, so these stay reactive to edits.
  const pictures = () => props.draft.pictures;
  const options = () => props.draft.options;
  const sequence = () => props.draft.sequence;
  const first = () => pictures()[0];

  const thumbnail = createThumbnails(() => pictures().map((p) => p.path), THUMB_HEIGHT);
  const order = createMemo(() => playbackOrder(pictures().length, options().direction));
  const loopSecs = () => order().length * sequence().frameSecs;
  const frame = createMemo(() => {
    const picture = first();
    return picture ? sequenceFrame(picture, options()) : null;
  });
  const totalBytes = () => pictures().reduce((sum, p) => sum + p.sizeBytes, 0);

  // Preview playback ----------------------------------------------------------

  const reducedMotion = createMediaQuery("(prefers-reduced-motion: reduce)");
  const [playing, setPlaying] = createSignal(!reducedMotion());
  const [step, setStep] = createSignal(0);
  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(
    on([playing, () => sequence().frameSecs, () => order().length], ([play, secs, count]) => {
      clearTimeout(timer);
      if (step() >= count) setStep(0);
      if (!play || count === 0) return;
      const tick = () => {
        setStep((s) => (s + 1) % count);
        timer = setTimeout(tick, secs * 1000);
      };
      timer = setTimeout(tick, secs * 1000);
    }),
  );
  onCleanup(() => clearTimeout(timer));
  const shownIndex = () => order()[step()] ?? 0;
  const shown = () => pictures()[shownIndex()];

  // Without the user's turn: the stage is turned as a whole by CSS.
  const unturned = createMemo(() => {
    const picture = first();
    return picture ? sequenceFrame(picture, { ...options(), rotate: 0 }) : null;
  });
  const quarterTurn = () => options().rotate % 180 === 90;
  const [spin, setSpin] = createSignal(options().rotate as number);
  const rotate = () => {
    setSpin((s) => s + 90);
    updateSequenceOptions({
      rotate: ((options().rotate + 90) % 360) as ConvertOptions["rotate"],
    });
  };

  // Filmstrip reordering ------------------------------------------------------

  const [dragging, setDragging] = createSignal<number | null>(null);
  const beginDrag = (index: number, event: PointerEvent) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
    const startX = event.clientX;
    const startY = event.clientY;
    let current = index;
    let moved = false;
    // Listen on the window: reordering moves the tile in the DOM, which
    // would drop a pointer capture.
    const move = (e: PointerEvent) => {
      if (!moved && Math.hypot(e.clientX - startX, e.clientY - startY) < 5) return;
      if (!moved) {
        moved = true;
        setDragging(current);
      }
      e.preventDefault();
      const over = document
        .elementFromPoint(e.clientX, e.clientY)
        ?.closest<HTMLElement>("[data-picture]");
      const target = Number(over?.getAttribute("data-picture") ?? Number.NaN);
      if (Number.isInteger(target) && target !== current) {
        movePicture(current, target);
        current = target;
        setDragging(current);
      }
    };
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      setDragging(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  };

  let strip: HTMLOListElement | undefined;
  const focusTile = (index: number) =>
    queueMicrotask(() => strip?.querySelector<HTMLElement>(`[data-picture="${index}"]`)?.focus());
  const onTileKey = (index: number, event: KeyboardEvent) => {
    const last = pictures().length - 1;
    const to =
      event.key === "ArrowLeft" || event.key === "ArrowUp"
        ? index - 1
        : event.key === "ArrowRight" || event.key === "ArrowDown"
          ? index + 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : null;
    if (to !== null) {
      event.preventDefault();
      const target = Math.min(last, Math.max(0, to));
      movePicture(index, target);
      focusTile(target);
    } else if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      removePicture(index);
      focusTile(Math.min(index, pictures().length - 1));
    }
  };

  // Settings ------------------------------------------------------------------

  const sizes = createMemo(() => {
    const picture = first();
    return picture ? sizeChoices(picture) : [];
  });
  const autoSize = () => {
    const cap = sequenceAutoSize(options().format);
    const picture = first();
    return cap !== null && picture && (shortSide(picture) ?? 0) > cap ? cap : null;
  };
  const isJxl = () => options().format === "jxl";
  const lossless = () => isLossless(options());
  const setJxl = (patch: Partial<ConvertOptions["jxl"]>) =>
    updateSequenceOptions({ jxl: { ...options().jxl, ...patch } });
  const estimate = createMemo(() => describeSequenceEstimate(pictures(), options()));
  // A slideshow's CRF from the core (pictures have no sound), and what it
  // comes to on average. Slideshows always use a CRF.
  const [encoding, setEncoding] = createSignal<QualitySummary | null>(null);
  let encodingAsked = 0;
  createEffect(
    on(
      () => [first(), JSON.stringify(options())] as const,
      ([picture]) => {
        const asked = ++encodingAsked;
        if (!picture) {
          setEncoding(null);
          return;
        }
        const snapshot = { ...options(), videoBitrate: null };
        Promise.resolve()
          .then(() => qualitySummary(picture, snapshot))
          .then(
            (summary) => asked === encodingAsked && setEncoding(summary ?? null),
            () => asked === encodingAsked && setEncoding(null),
          );
      },
    ),
  );
  const qualityNumbers = createMemo(() =>
    describeQuality(encoding(), estimateSequenceVideoBitrate(pictures(), options(), sequence())),
  );
  // Where it's saved ----------------------------------------------------------

  const target = () => targetOf({ ...props.draft.output });
  const [saveAt, setSaveAt] = createSignal<OutputPreview | null>(null);
  const [outputError, setOutputError] = createSignal<string | null>(null);
  let outputTimer: ReturnType<typeof setTimeout> | undefined;
  let outputChecks = 0;
  createEffect(
    on(
      () => JSON.stringify([pictures().map((p) => p.path), options(), target()]),
      () => {
        clearTimeout(outputTimer);
        const check = ++outputChecks;
        const snapshot = { pictures: [...pictures()], options: { ...options() }, to: target() };
        outputTimer = setTimeout(() => {
          sequenceOutputPreview(snapshot.pictures, snapshot.options, snapshot.to).then(
            (found) => {
              if (check !== outputChecks) return;
              setSaveAt(found);
              setOutputError(null);
            },
            (error) => {
              if (check !== outputChecks) return;
              setSaveAt(null);
              setOutputError(String(error));
            },
          );
        }, 150);
      },
    ),
  );
  onCleanup(() => clearTimeout(outputTimer));

  const ready = () => {
    const codec = codecOf(options());
    return (
      state.ffmpeg?.found === true &&
      isAnimated(options().format) &&
      (codec === null || isCodecAvailable(codec)) &&
      outputError() === null
    );
  };
  const unavailableReason = (format: ConvertOptions["format"]) =>
    format === "jxl"
      ? "Install libjxl (brew install jpeg-xl) or an ffmpeg with libjxl_anim for animated JXL"
      : format === "webp"
        ? "Your ffmpeg can't write animated WebP (it needs libwebp_anim)"
        : `Your ffmpeg can't write ${label(format)}`;

  const summary = createMemo(() => {
    const items: { icon: "image" | "clock" | "refresh" | "rotate" | "flip"; text: string }[] = [];
    items.push({ icon: "image", text: `${order().length} frames` });
    items.push({ icon: "clock", text: `${formatSpan(loopSecs())} loop` });
    const f = frame();
    if (f) items.push({ icon: "image", text: `${f.width}×${f.height}` });
    if (options().direction !== "forward") {
      items.push({
        icon: "refresh",
        text: options().direction === "boomerang" ? "Boomerang" : "Reversed",
      });
    }
    if (options().rotate) items.push({ icon: "rotate", text: `Turned ${options().rotate}°` });
    if (options().flip) items.push({ icon: "flip", text: "Mirrored" });
    return items;
  });

  // Command preview -----------------------------------------------------------

  const [command, setCommand] = createSignal("");
  const [showCommand, setShowCommand] = createSignal(false);
  let commandTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(
    on(
      () =>
        JSON.stringify([
          pictures().map((p) => p.path),
          options(),
          sequence(),
          target(),
          showCommand(),
        ]),
      () => {
        clearTimeout(commandTimer);
        if (!showCommand()) return;
        const snapshot = {
          pictures: [...pictures()],
          options: { ...options() },
          sequence: { ...sequence() } as SequenceOptions,
          target: target(),
        };
        commandTimer = setTimeout(() => {
          previewSequenceCommand(
            snapshot.pictures,
            snapshot.options,
            snapshot.sequence,
            snapshot.target,
          ).then(setCommand, (error) => setCommand(`# ${error}`));
        }, 150);
      },
    ),
  );
  onCleanup(() => clearTimeout(commandTimer));

  const convertLabel = () => (state.inbox.length > 0 ? "Make it & next" : "Make animation");
  const convertButton = (cls: string) => (
    <button
      type="button"
      class={`btn btn-primary ${cls}`}
      disabled={!ready()}
      onClick={() => void convertSequence()}
    >
      <Icon name="sparkle" size={18} />
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
          <h1 class="display file-title">{pictures().length} pictures</h1>
          <p class="item-meta">
            Animation · {formatSpan(loopSecs())} loop · {formatBytes(totalBytes())}
          </p>
        </div>
        <Show when={!props.compact}>{convertButton("btn-lg")}</Show>
      </header>

      <div class="file-grid">
        <div class="stack">
          <div class="preview sequence-preview">
            <Show when={unturned()}>
              {(size) => (
                <div
                  class="sequence-stage"
                  classList={{ quarter: quarterTurn() }}
                  data-fit={sequence().fit}
                  style={{
                    "--ratio": `${size().width / size().height}`,
                    transform: `rotate(${spin()}deg) scaleX(${options().flip ? -1 : 1})`,
                  }}
                >
                  <Show when={shown() && thumbnail(shown()?.path ?? "")}>
                    {(src) => (
                      <img
                        class="sequence-frame"
                        src={src()}
                        alt={`Frame ${step() + 1} of ${order().length}: ${shown()?.fileName}`}
                        draggable={false}
                      />
                    )}
                  </Show>
                </div>
              )}
            </Show>
            <span class="preview-badge mono">
              {step() + 1} / {order().length}
            </span>
            <div class="preview-tools">
              <button
                type="button"
                class="tool-btn"
                aria-label={playing() ? "Pause preview" : "Play preview"}
                title={playing() ? "Pause" : "Play"}
                onClick={() => setPlaying(!playing())}
              >
                <Icon name={playing() ? "pause" : "play"} size={20} />
              </button>
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
                onClick={() => updateSequenceOptions({ flip: !options().flip })}
              >
                <Icon name="flip" size={20} />
              </button>
            </div>
          </div>

          <section class="card pictures">
            <div class="row-between">
              <h2 class="label-strong">
                <Icon name="image" size={16} /> Pictures
              </h2>
              <button type="button" class="link-btn" onClick={() => sortPictures()}>
                Sort by name
              </button>
            </div>
            <p class="item-meta" id="filmstrip-help">
              Drag to reorder, or focus one and use the arrow keys. Delete leaves it out.
            </p>
            <ol
              ref={strip}
              class="picture-strip"
              aria-label="Pictures in order"
              aria-describedby="filmstrip-help"
            >
              <For each={pictures()}>
                {(picture, index) => (
                  <li
                    class="picture-tile"
                    classList={{
                      dragging: dragging() === index(),
                      showing: shownIndex() === index(),
                    }}
                    data-picture={index()}
                    tabindex="0"
                    aria-label={`${index() + 1}. ${picture.fileName}`}
                    title={picture.fileName}
                    onPointerDown={(e) => beginDrag(index(), e)}
                    onKeyDown={(e) => onTileKey(index(), e)}
                  >
                    <Show when={thumbnail(picture.path)} fallback={<span class="tile-skeleton" />}>
                      {(src) => <img src={src()} alt="" draggable={false} />}
                    </Show>
                    <span class="picture-number">{index() + 1}</span>
                    <button
                      type="button"
                      class="picture-remove"
                      aria-label={`Leave out ${picture.fileName}`}
                      tabindex="-1"
                      onClick={() => removePicture(index())}
                    >
                      <Icon name="close" size={12} />
                    </button>
                  </li>
                )}
              </For>
            </ol>
          </section>
        </div>

        <section class="card settings">
          <fieldset class="field">
            <legend class="field-label">Turn them into</legend>
            <div class="format-chips" data-tone="pink">
              <For each={SEQUENCE_FORMATS}>
                {(format) => (
                  <button
                    type="button"
                    class="format-chip"
                    aria-pressed={options().format === format}
                    disabled={!isAnimated(format)}
                    title={isAnimated(format) ? undefined : unavailableReason(format)}
                    onClick={() => updateSequenceOptions({ format })}
                  >
                    {label(format)}
                  </button>
                )}
              </For>
            </div>
          </fieldset>

          <CodecField
            format={options().format}
            codec={options().videoCodec ?? null}
            available={isCodecAvailable}
            onChange={(videoCodec) => updateSequenceOptions({ videoCodec })}
          />

          <fieldset class="field">
            <legend class="field-label">Each picture shows for</legend>
            <div class="format-chips" data-tone="lavender">
              <For each={FRAME_TIMES}>
                {(secs) => (
                  <button
                    type="button"
                    class="format-chip"
                    aria-pressed={sequence().frameSecs === secs}
                    onClick={() => updateSequence({ frameSecs: secs })}
                  >
                    {secondsLabel(secs)}
                  </button>
                )}
              </For>
            </div>
          </fieldset>

          <fieldset class="field">
            <legend class="field-label">Playback</legend>
            <div class="format-chips" data-tone="green">
              <For each={DIRECTIONS}>
                {(choice) => (
                  <button
                    type="button"
                    class="format-chip"
                    aria-pressed={options().direction === choice.direction}
                    onClick={() => updateSequenceOptions({ direction: choice.direction })}
                  >
                    {choice.label}
                  </button>
                )}
              </For>
            </div>
          </fieldset>

          <fieldset class="field">
            <legend class="field-label">Other shapes fit the first picture with</legend>
            <div class="format-chips" data-tone="yellow">
              <For each={FITS}>
                {(choice) => (
                  <button
                    type="button"
                    class="format-chip"
                    aria-pressed={sequence().fit === choice.value}
                    title={choice.hint}
                    onClick={() => updateSequence({ fit: choice.value })}
                  >
                    {choice.label}
                  </button>
                )}
              </For>
            </div>
          </fieldset>

          <Show when={first()?.width}>
            <label class="field">
              <span class="field-label">Size</span>
              <select
                class="select"
                onChange={(e) => {
                  const v = e.currentTarget.value;
                  updateSequenceOptions({ maxSize: v === "" ? null : Number(v) });
                }}
              >
                <option value="" selected={options().maxSize === null}>
                  {autoSize() !== null
                    ? `Auto (${autoSize()} px)`
                    : `Like the first (${first()?.width}×${first()?.height})`}
                </option>
                <For each={sizes()}>
                  {(size) => (
                    <option value={size} selected={options().maxSize === size}>
                      {size} px
                    </option>
                  )}
                </For>
              </select>
            </label>
          </Show>

          <Show
            when={!lossless()}
            fallback={
              <p class="lossless-note">
                <Icon name="sparkle" size={16} />
                Lossless: every pixel stays put.
              </p>
            }
          >
            <QualityField
              quality={options().quality}
              numbers={qualityNumbers()}
              describedBy={estimate() ? "sequence-estimate" : undefined}
              onChange={(quality) => updateSequenceOptions({ quality })}
            />
          </Show>

          <Show when={estimate()}>
            {(text) => (
              <p
                id="sequence-estimate"
                class="estimate"
                title="A rough guess from the pictures and these settings. The real size depends on what's in them."
              >
                {text()}
              </p>
            )}
          </Show>

          <Show when={isJxl()}>
            <div class="jxl-settings">
              <div class="toggle-row">
                <div>
                  <span class="toggle-title" id="jxl-lossless">
                    Lossless
                  </span>
                  <span class="item-meta">Keep every pixel exactly</span>
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
            </div>
          </Show>

          <div class="outcome">
            <OutputField
              choice={props.draft.output}
              preview={saveAt()}
              error={outputError()}
              format={options().format}
              formatLabel={label(options().format)}
              usualName={sequenceName(pictures())}
              usualPlace="next to the first picture"
              files={1}
            />
            <ul class="outcome-chips" aria-label="Animation">
              <For each={summary()}>
                {(item) => (
                  <li class="outcome-chip">
                    <Icon name={item.icon} size={14} />
                    {item.text}
                  </li>
                )}
              </For>
            </ul>
          </div>

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
