import { type Component, For } from "solid-js";
import { QUALITY_STEPS, qualityStep } from "../lib/formats";

type Props = {
  quality: number;
  onChange: (quality: number) => void;
  /** Id of the element describing the result, such as the size estimate. */
  describedBy?: string | undefined;
};

/** The quality slider from "Tiny file" to "Crisp". */
export const QualityField: Component<Props> = (props) => (
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
      aria-describedby={props.describedBy}
      value={props.quality}
      style={{ "--pct": `${props.quality}%` }}
      onInput={(event) => props.onChange(event.currentTarget.valueAsNumber)}
    />
    <div class="range-scale">
      <For each={QUALITY_STEPS}>
        {(name) => <span classList={{ active: qualityStep(props.quality) === name }}>{name}</span>}
      </For>
    </div>
  </div>
);
