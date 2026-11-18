import { type Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { compareFile, compareSample } from "../lib/api";
import {
  convertedName,
  createSample,
  originalLabel,
  PREVIEW_LIMIT,
  sourceTime,
  type Zoom,
} from "../lib/compare";
import { pictureSize } from "../lib/crop";
import { baseName, formatBytes, formatClock } from "../lib/format";
import { formatKind, isAnimatedImage, outputSeconds } from "../lib/formats";
import { FALLBACK_FPS } from "../lib/frames";
import { type Comparison, closeComparison, setCompareLayout, state } from "../lib/store";
import { CompareControls } from "./CompareControls";
import { CompareView } from "./CompareView";
import { Icon } from "./Icon";

/**
 * A finished file next to its original, opened from the Queue or History.
 * The original gets the same turn, mirror and crop as the conversion, so
 * only what the encoding changed shows. Videos get a slider to pick the
 * moment compared.
 */
export const ComparisonSheet: Component<{ comparison: Comparison; compact: boolean }> = (props) => {
  // A comparison is replaced rather than changed, so these are fixed.
  const { input, options, output, outputBytes, name } = props.comparison;
  const out = formatKind(options.format, input.kind);
  /** The result plays: a video or an animation made from one. */
  const timed = input.kind === "video" && out === "video";
  const length = (timed ? outputSeconds(input, options) : null) ?? 0;
  const fps = Math.min(input.fps ?? FALLBACK_FPS, options.fps ?? Number.POSITIVE_INFINITY);
  const hdr =
    input.hdr &&
    out === "video" &&
    options.format !== "gif" &&
    !isAnimatedImage(options.format, input.kind);

  const [zoom, setZoom] = createSignal<Zoom>("fit");
  const [at, setAt] = createSignal(timed ? Math.min(1, length * 0.15) : 0);
  const limit = () => (zoom() === "fit" ? PREVIEW_LIMIT : null);
  /** Where in the original the moment compared is; a picture from a video is its trim start. */
  const source = () =>
    timed
      ? sourceTime(options, input.durationSecs, at())
      : input.kind === "video"
        ? (options.trimStart ?? 0)
        : 0;

  const before = createSample(() => {
    const [t, max] = [source(), limit()];
    return {
      key: JSON.stringify([t, max]),
      run: () => compareSample(input, options, "original", t, max),
    };
  }, 80);
  const after = createSample(() => {
    const [t, max] = [at(), limit()];
    return { key: JSON.stringify([t, max]), run: () => compareFile(output, t, max, hdr) };
  }, 80);

  const mode = () => (props.compact ? "flip" : state.settings.compareLayout);

  let closeButton: HTMLButtonElement | undefined;
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  onMount(() => closeButton?.focus());
  onCleanup(() => opener?.focus());
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeComparison();
    }
  };
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <div
      class="sheet-backdrop"
      onPointerDown={(e) => e.target === e.currentTarget && closeComparison()}
    >
      <div
        class="sheet compare-sheet"
        classList={{ wide: mode() === "side" }}
        role="dialog"
        aria-modal="true"
        aria-labelledby="comparison-title"
      >
        <header class="sheet-header">
          <div class="sheet-heading">
            <h2 class="display sheet-title" id="comparison-title" title={name}>
              {name}
            </h2>
            <p class="item-meta">Next to {baseName(input.path)}</p>
          </div>
          <button
            ref={closeButton}
            type="button"
            class="icon-btn plain"
            aria-label="Close"
            onClick={() => closeComparison()}
          >
            <Icon name="close" size={20} />
          </button>
        </header>
        <div class="preview comparing" classList={{ "compare-side": mode() === "side" }}>
          <CompareView
            before={{
              state: before.state(),
              label: originalLabel(input),
              onBroken: before.retry,
            }}
            after={{
              state: after.state(),
              label: `${convertedName(input, options)} · ${formatBytes(outputBytes)}`,
              onBroken: after.retry,
            }}
            picture={pictureSize(input, options)}
            mode={mode()}
            zoom={zoom()}
            onZoom={setZoom}
            name={name}
          />
          <div class="preview-tools">
            <CompareControls
              zoom={zoom()}
              onZoom={setZoom}
              layout={props.compact ? null : state.settings.compareLayout}
              onLayout={setCompareLayout}
            />
          </div>
        </div>
        <Show when={timed && length > 0}>
          <label class="compare-time">
            <span class="field-label">Moment</span>
            <input
              type="range"
              class="compare-range"
              min={0}
              max={Math.max(0, length - 1 / fps)}
              step={1 / fps}
              value={at()}
              onInput={(e) => setAt(Number(e.currentTarget.value))}
            />
            <span class="mono item-meta">{formatClock(at(), true)}</span>
          </label>
        </Show>
      </div>
    </div>
  );
};
