import { type Component, For, Show } from "solid-js";
import { type CompareLayout, ZOOMS, type Zoom } from "../lib/compare";
import { Icon } from "./Icon";

/**
 * The comparison's zoom (fit, 100%, 200%) and, on wide windows, whether the
 * halves wipe into each other or sit side by side. Floats over the preview,
 * next to the compare switch.
 */
export const CompareControls: Component<{
  zoom: Zoom;
  onZoom: (zoom: Zoom) => void;
  /** `null` hides the layout choice (phones flip between the halves instead). */
  layout: CompareLayout | null;
  onLayout: (layout: CompareLayout) => void;
}> = (props) => (
  <>
    <fieldset class="compare-seg" aria-label="Zoom">
      <For each={ZOOMS}>
        {(choice) => (
          <button
            type="button"
            class="compare-chip"
            aria-pressed={props.zoom === choice.zoom}
            onClick={() => props.onZoom(choice.zoom)}
          >
            {choice.label}
          </button>
        )}
      </For>
    </fieldset>
    <Show when={props.layout}>
      {(layout) => (
        <fieldset class="compare-seg" aria-label="Layout">
          <button
            type="button"
            class="compare-chip icon"
            aria-label="Wipe"
            title="Wipe between them"
            aria-pressed={layout() === "wipe"}
            onClick={() => props.onLayout("wipe")}
          >
            <Icon name="wipe" size={17} />
          </button>
          <button
            type="button"
            class="compare-chip icon"
            aria-label="Side by side"
            title="Side by side"
            aria-pressed={layout() === "side"}
            onClick={() => props.onLayout("side")}
          >
            <Icon name="columns" size={17} />
          </button>
        </fieldset>
      )}
    </Show>
  </>
);
