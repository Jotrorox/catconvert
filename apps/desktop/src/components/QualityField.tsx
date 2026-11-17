import { type Component, For, Show } from "solid-js";
import { formatBitrate } from "../lib/format";
import { MAX_VIDEO_KBPS, MIN_VIDEO_KBPS, QUALITY_STEPS, qualityStep } from "../lib/formats";

type Props = {
  quality: number;
  onChange: (quality: number) => void;
  /** The quality in numbers, e.g. "Audio 128 kbps · Video ~3.8 Mbps (CRF 24)". */
  numbers?: string | null | undefined;
  /** Id of the element describing the result, such as the size estimate. */
  describedBy?: string | undefined;
  /** Exact video bitrate in kbit/s, or `null` when the quality level decides. */
  bitrate?: number | null | undefined;
  /** Offers the exact-bitrate mode; called with `null` to go back to levels. */
  onBitrate?: ((kbps: number | null) => void) | undefined;
  /** Where the exact bitrate starts, in kbit/s: about what the level gives now. */
  suggestedBitrate?: (() => number) | undefined;
};

/** Kbit/s to the Mbps the field shows, to a tenth (or a hundredth below 1 Mbps). */
function toMbps(kbps: number): string {
  const mbps = kbps / 1000;
  return String(mbps < 1 ? Math.round(mbps * 100) / 100 : Math.round(mbps * 10) / 10);
}

/** What was typed in the Mbps field as kbit/s within range, or `null` if it isn't a number. */
export function parseMbps(text: string): number | null {
  const mbps = Number(text.trim().replace(",", "."));
  if (text.trim() === "" || !Number.isFinite(mbps)) return null;
  return Math.min(MAX_VIDEO_KBPS, Math.max(MIN_VIDEO_KBPS, Math.round(mbps * 1000)));
}

/**
 * The quality slider from "Tiny file" to "Crisp", with what it means in
 * bitrates underneath. For video files it can switch to an exact bitrate.
 */
export const QualityField: Component<Props> = (props) => {
  const exact = () => props.onBitrate !== undefined && (props.bitrate ?? null) !== null;
  const describedBy = () =>
    [props.numbers ? "quality-numbers" : null, props.describedBy].filter(Boolean).join(" ") ||
    undefined;
  const commit = (input: HTMLInputElement) => {
    const kbps = parseMbps(input.value);
    if (kbps === null) {
      input.value = toMbps(props.bitrate ?? MIN_VIDEO_KBPS);
      return;
    }
    input.value = toMbps(kbps);
    props.onBitrate?.(kbps);
  };
  const choose = (wantExact: boolean) => {
    if (wantExact === exact()) return;
    const suggested = props.suggestedBitrate?.() ?? 4000;
    const kbps = Math.min(MAX_VIDEO_KBPS, Math.max(MIN_VIDEO_KBPS, Math.round(suggested)));
    props.onBitrate?.(wantExact ? kbps : null);
  };

  return (
    <div class="field">
      <div class="quality-head">
        <label class="field-label" for={exact() ? "video-bitrate" : "quality"}>
          {exact() ? "Video bitrate" : "Quality"}
        </label>
        <Show when={props.onBitrate}>
          <fieldset class="format-chips quality-modes" aria-label="Set quality by">
            <button
              type="button"
              class="format-chip"
              aria-pressed={!exact()}
              onClick={() => choose(false)}
            >
              Level
            </button>
            <button
              type="button"
              class="format-chip"
              aria-pressed={exact()}
              title="Aim for an exact average bitrate instead of a quality level"
              onClick={() => choose(true)}
            >
              Bitrate
            </button>
          </fieldset>
        </Show>
      </div>
      <Show
        when={exact()}
        fallback={
          <>
            <input
              id="quality"
              class="range"
              type="range"
              min="0"
              max="100"
              aria-describedby={describedBy()}
              value={props.quality}
              style={{ "--pct": `${props.quality}%` }}
              onInput={(event) => props.onChange(event.currentTarget.valueAsNumber)}
            />
            <div class="range-scale">
              <For each={QUALITY_STEPS}>
                {(name) => (
                  <span classList={{ active: qualityStep(props.quality) === name }}>{name}</span>
                )}
              </For>
            </div>
          </>
        }
      >
        <div class="bitrate-row">
          <input
            id="video-bitrate"
            class="input bitrate-input"
            type="text"
            inputmode="decimal"
            aria-describedby={describedBy()}
            value={toMbps(props.bitrate ?? MIN_VIDEO_KBPS)}
            onChange={(event) => commit(event.currentTarget)}
            onKeyDown={(event) => event.key === "Enter" && commit(event.currentTarget)}
          />
          <span class="bitrate-unit">Mbps</span>
        </div>
        <span class="item-meta">
          Between {formatBitrate(MIN_VIDEO_KBPS * 1000)} and {formatBitrate(MAX_VIDEO_KBPS * 1000)}.
          The sound keeps its level.
        </span>
      </Show>
      <Show when={props.numbers}>
        {(text) => (
          <p
            id="quality-numbers"
            class="quality-numbers"
            title="The audio bitrate and CRF are exactly what the command uses; a CRF's bitrate depends on the picture, so it is an estimate."
          >
            {text()}
          </p>
        )}
      </Show>
    </div>
  );
};
