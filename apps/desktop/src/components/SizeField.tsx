import { type Component, createEffect, createMemo, createSignal, For, on, Show } from "solid-js";
import type { ConvertOptions, Scaler, Upscale } from "../lib/api";
import type { Size } from "../lib/crop";
import { SCALERS, sameUpscale } from "../lib/formats";
import { Icon } from "./Icon";

/** What the Size select can pick: a shrink cap, an upscale preset or a custom upscale. */
export type SizePick =
  | { maxSize: number | null; upscale?: undefined }
  | { upscale: Upscale; maxSize?: undefined }
  | "custom";

/** Select value of an upscale preset: `x2` for 2×, `s1080` for 1080p. */
const presetValue = (upscale: Upscale) =>
  "factor" in upscale ? `x${upscale.factor}` : `s${upscale.size}`;

type SelectProps = {
  /** The first choice, e.g. "Original (1080p)" or "Auto (480p)". */
  keepLabel: string;
  /** Short sides to shrink to. */
  shrinkSizes: readonly number[];
  sizeText: (size: number) => string;
  /** Upscale presets that fit this file. */
  presets: readonly Upscale[];
  presetText: (upscale: Upscale) => string;
  /** A custom upscale can make it bigger at all. */
  canCustom: boolean;
  maxSize: number | null;
  /** The upscale while it enlarges the picture, otherwise `null`. */
  upscale: Upscale | null;
  /** "Custom…" was picked, even if the value matches a preset. */
  custom: boolean;
  onPick: (pick: SizePick) => void;
};

/**
 * The Size select: keep the size, shrink it (the default choices), or under
 * "Upscale" make it bigger by a factor, to a target or by a custom amount.
 */
export const SizeSelect: Component<SelectProps> = (props) => {
  const value = () => {
    const upscale = props.upscale;
    if (props.custom) return "custom";
    if (upscale) {
      const preset = props.presets.find((p) => sameUpscale(p, upscale));
      return preset ? presetValue(preset) : "custom";
    }
    return props.maxSize !== null && props.shrinkSizes.includes(props.maxSize)
      ? String(props.maxSize)
      : "";
  };
  const pick = (v: string) => {
    if (v === "custom") return props.onPick("custom");
    const preset = props.presets.find((p) => presetValue(p) === v);
    if (preset) return props.onPick({ upscale: preset });
    props.onPick({ maxSize: v === "" ? null : Number(v) });
  };
  return (
    <label class="field">
      <span class="field-label">Size</span>
      <select class="select" onChange={(e) => pick(e.currentTarget.value)}>
        <option value="" selected={value() === ""}>
          {props.keepLabel}
        </option>
        <For each={props.shrinkSizes}>
          {(size) => (
            <option value={size} selected={value() === String(size)}>
              {props.sizeText(size)}
            </option>
          )}
        </For>
        <Show when={props.presets.length > 0 || props.canCustom}>
          <optgroup label="Upscale">
            <For each={props.presets}>
              {(preset) => (
                <option value={presetValue(preset)} selected={value() === presetValue(preset)}>
                  {props.presetText(preset)}
                </option>
              )}
            </For>
            <Show when={props.canCustom}>
              <option value="custom" selected={value() === "custom"}>
                Custom…
              </option>
            </Show>
          </optgroup>
        </Show>
      </select>
    </label>
  );
};

type SettingsProps = {
  /** Show the custom short-side field. */
  custom: boolean;
  /** Smallest and biggest short side a custom upscale can ask for. */
  minSize: number;
  maxSize: number;
  /** The short side the upscale reaches, for the custom field. */
  target: number | null;
  /** The size that gets written. */
  result: Size | null;
  /** The picture gets bigger (and the scaler matters). */
  enlarged: boolean;
  scaler: Scaler;
  /** The result is big enough to warn about. */
  huge: boolean;
  /** The result is more than the format can hold: its long-side limit and name. */
  tooBig: { limit: number; format: string } | null;
  onChange: (patch: Partial<ConvertOptions>) => void;
};

const sizeText = (size: Size) => `${size.width}×${size.height}`;

/**
 * What goes with an upscale: the custom target, the scaler, and a word when
 * the result gets huge or too big to write.
 */
export const UpscaleSettings: Component<SettingsProps> = (props) => {
  const [note, setNote] = createSignal<string | null>(null);
  // A clamp note is about the value just typed, not a later one.
  // Memoised, so edits that keep the same bounds leave it be.
  const bounds = createMemo(() => `${props.custom} ${props.minSize} ${props.maxSize}`);
  createEffect(
    on(bounds, () => setNote(null), {
      defer: true,
    }),
  );
  let input: HTMLInputElement | undefined;
  const commit = () => {
    if (!input) return;
    const typed = Number(input.value.trim());
    if (input.value.trim() === "" || !Number.isFinite(typed)) {
      input.value = String(props.target ?? props.minSize);
      setNote(`Type a short side between ${props.minSize} and ${props.maxSize} px.`);
      return;
    }
    const size = Math.min(props.maxSize, Math.max(props.minSize, Math.round(typed)));
    if (size < typed) setNote(`${props.maxSize} px is as big as this file can go.`);
    else if (size > typed) setNote(`Upscaling only makes it bigger: ${props.minSize} px at least.`);
    else setNote(null);
    input.value = String(size);
    props.onChange({ upscale: { size }, maxSize: null });
  };

  return (
    <Show when={props.custom || props.enlarged || props.tooBig}>
      <div class="upscale-settings">
        <div class="two-col">
          <Show when={props.custom}>
            <div class="field">
              <label class="field-label" for="upscale-size">
                Short side
              </label>
              <div class="bitrate-row">
                <input
                  ref={input}
                  id="upscale-size"
                  class="input bitrate-input"
                  type="text"
                  inputmode="numeric"
                  aria-describedby="upscale-result"
                  value={props.target ?? props.minSize}
                  onChange={commit}
                  onKeyDown={(event) => event.key === "Enter" && commit()}
                />
                <span class="bitrate-unit">px</span>
              </div>
              <Show when={props.result}>
                {(size) => (
                  <span id="upscale-result" class="item-meta">
                    Makes it {sizeText(size())}
                  </span>
                )}
              </Show>
            </div>
          </Show>
          <Show when={props.enlarged}>
            <label class="field">
              <span class="field-label">Scaler</span>
              <select
                class="select"
                onChange={(e) => props.onChange({ scaler: e.currentTarget.value as Scaler })}
              >
                <For each={SCALERS}>
                  {(choice) => (
                    <option
                      value={choice.scaler}
                      selected={
                        props.scaler === choice.scaler ||
                        (props.scaler === "lanczos" && choice.scaler === "auto")
                      }
                    >
                      {choice.label}
                    </option>
                  )}
                </For>
              </select>
            </label>
          </Show>
        </div>
        <Show when={note()}>
          {(text) => (
            <span class="item-meta" role="status">
              {text()}
            </span>
          )}
        </Show>
        <Show
          when={props.tooBig}
          fallback={
            <Show when={props.enlarged}>
              <Show
                when={props.huge && props.result}
                fallback={<span class="item-meta">Upscaling makes it bigger, not sharper.</span>}
              >
                {(size) => (
                  <p class="lossless-note warning-note" role="status">
                    <Icon name="alert" size={16} />
                    That's {sizeText(size())}: encoding will be slow and the file big.
                  </p>
                )}
              </Show>
            </Show>
          }
        >
          {(over) => (
            <p class="lossless-note warning-note" role="alert">
              <Icon name="alert" size={16} />
              {props.result ? `That's ${sizeText(props.result)}, too big` : "Too big"} for{" "}
              {over().format}, which stops at {over().limit} px on the long side. Pick a smaller
              size.
            </p>
          )}
        </Show>
      </div>
    </Show>
  );
};
