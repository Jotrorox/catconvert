import { type Component, For, Show } from "solid-js";
import type { Crop, CropArea } from "../lib/api";
import { type Corner, keptArea, moveArea, resizeArea, type Size } from "../lib/crop";

type Props = {
  /** The picture's size in pixels after the turn. */
  picture: Size;
  /** Where the picture sits on screen, in CSS pixels. */
  shown: Size;
  crop: Crop | null;
  /** Handles and grid while editing; just the shaded result otherwise. */
  editing: boolean;
  /** Fade in late, once the picture has finished turning. */
  settle?: boolean;
  onChange: (crop: Crop) => void;
  onDone: () => void;
};

const CORNERS: readonly Corner[] = ["nw", "ne", "sw", "se"];

/**
 * The crop frame drawn over the preview. Drag inside to move it, drag a
 * corner to resize it; arrow keys nudge it (Shift for bigger steps).
 */
export const CropOverlay: Component<Props> = (props) => {
  let layer: HTMLDivElement | undefined;
  const area = () => keptArea(props.picture, props.crop);
  const aspect = () => props.crop?.aspect ?? null;
  const set = (next: CropArea) => props.onChange({ aspect: aspect(), area: next });

  const drag = (event: PointerEvent, corner: Corner | null) => {
    if (!props.editing || event.button !== 0 || !layer) return;
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const rect = layer.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const start = area();
    const [x0, y0] = [event.clientX, event.clientY];
    layer.classList.add("dragging");

    const move = (e: PointerEvent) => {
      if (corner) {
        const point = {
          x: (e.clientX - rect.left) / rect.width,
          y: (e.clientY - rect.top) / rect.height,
        };
        set(resizeArea(props.picture, aspect(), start, corner, point));
      } else {
        set(moveArea(start, (e.clientX - x0) / rect.width, (e.clientY - y0) / rect.height));
      }
    };
    const up = () => {
      layer?.classList.remove("dragging");
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", up);
  };

  const onKey = (event: KeyboardEvent) => {
    const step = event.shiftKey ? 0.1 : 0.01;
    const nudge: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const delta = nudge[event.key];
    if (delta) set(moveArea(area(), delta[0], delta[1]));
    else if (event.key === "Enter" || event.key === "Escape") props.onDone();
    else return;
    event.preventDefault();
  };

  const pct = (v: number) => `${v * 100}%`;
  const place = () => ({
    left: pct(area().x),
    top: pct(area().y),
    width: pct(area().width),
    height: pct(area().height),
  });

  return (
    <div
      ref={layer}
      class="crop-layer"
      classList={{ editing: props.editing, settle: props.settle }}
      style={{ width: `${props.shown.width}px`, height: `${props.shown.height}px` }}
      aria-hidden={!props.editing}
    >
      <div class="crop-shade">
        <span class="crop-hole" style={place()} />
      </div>
      <button
        type="button"
        class="crop-frame"
        aria-label="Crop area: arrow keys move it, Enter finishes"
        tabIndex={props.editing ? 0 : -1}
        style={place()}
        onPointerDown={(e) => drag(e, null)}
        onKeyDown={onKey}
      >
        <Show when={props.editing}>
          <span class="crop-grid" />
          <For each={CORNERS}>
            {(corner) => (
              <span
                class={`crop-handle ${corner}`}
                onPointerDown={(e) => drag(e, corner)}
                aria-hidden="true"
              />
            )}
          </For>
        </Show>
      </button>
    </div>
  );
};
