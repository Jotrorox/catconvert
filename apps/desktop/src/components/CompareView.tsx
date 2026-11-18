import { type Component, createEffect, createSignal, on, onCleanup, Show } from "solid-js";
import {
  type CompareMode,
  clampPan,
  type Point,
  panAround,
  type SampleState,
  scaleFor,
  type Zoom,
} from "../lib/compare";
import type { Size } from "../lib/crop";
import { Icon } from "./Icon";

export type CompareHalf = {
  state: SampleState;
  /** "Original · 4.2 MB", "WebP q73 · ~610 KB". */
  label: string;
  /** The picture didn't load, e.g. because its file was cleaned up. */
  onBroken?: () => void;
};

type Props = {
  before: CompareHalf;
  after: CompareHalf;
  /** The compared picture's size in pixels (after the turn and crop), if known. */
  picture: Size | null;
  mode: CompareMode;
  zoom: Zoom;
  onZoom: (zoom: Zoom) => void;
  /** Names the pictures for screen readers. */
  name: string;
};

type Side = "before" | "after";

/** How far a press may move and still count as a tap (flip) rather than a pan. */
const TAP_SLOP = 5;

/** The longest gap between the taps of a double tap (or click). */
const DOUBLE_TAP_MS = 320;

/**
 * The original and the converted picture, compared. Both halves share one
 * zoom and pan, so the same detail is always side by side.
 *
 * - **wipe**: one picture with a divider; drag it (or anywhere, while the
 *   picture fits) to wipe between them. Arrow keys move it too.
 * - **side**: the two next to each other.
 * - **flip**: one at a time; tap to flip. For phones.
 *
 * Zoomed in, dragging (or scrolling) pans; a double click or tap toggles
 * 100%.
 */
export const CompareView: Component<Props> = (props) => {
  const [stage, setStage] = createSignal<Size>({ width: 0, height: 0 });
  const [divider, setDivider] = createSignal(0.5);
  const [pan, setPan] = createSignal<Point>({ x: 0, y: 0 });
  const [showing, setShowing] = createSignal<Side>("after");
  const [natural, setNatural] = createSignal<Size | null>(null);

  // Measure the first stage; side by side, both have its size.
  let observer: ResizeObserver | undefined;
  const watch = (el: HTMLDivElement) => {
    const measure = () => setStage({ width: el.clientWidth, height: el.clientHeight });
    observer?.disconnect();
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(measure);
      observer.observe(el);
    }
    queueMicrotask(measure);
  };
  onCleanup(() => observer?.disconnect());
  /** Set up a stage: scrolling pans it, and the first one is measured. */
  const stageRef = (first: boolean) => (el: HTMLDivElement) => {
    if (first) watch(el);
    el.addEventListener("wheel", scroll, { passive: false });
  };

  const picture = (): Size => {
    const known = props.picture ?? natural();
    return known && known.width > 0 && known.height > 0 ? known : stage();
  };
  const scale = () => scaleFor(props.zoom, stage(), picture());
  const shown = (): Size => ({
    width: picture().width * scale(),
    height: picture().height * scale(),
  });
  const zoomed = () => shown().width > stage().width + 0.5 || shown().height > stage().height + 0.5;
  const shownPan = () => clampPan(pan(), stage(), shown());

  // A new zoom keeps the point under the pointer (or the middle) in place.
  let anchor: Point = { x: 0, y: 0 };
  createEffect(
    on(
      scale,
      (now, before) => {
        if (before === undefined || props.zoom === "fit") setPan({ x: 0, y: 0 });
        else setPan(clampPan(panAround(shownPan(), anchor, before, now), stage(), shown()));
        anchor = { x: 0, y: 0 };
      },
      { defer: true },
    ),
  );

  // Two taps in quick succession toggle 100% around the spot tapped.
  let lastTap: { time: number; x: number; y: number } | null = null;

  const press = (event: PointerEvent) => {
    if (event.button !== 0) return;
    const el = event.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const wiping = props.mode === "wipe" && !zoomed();
    const wipeTo = (x: number) => {
      if (rect.width > 0) setDivider(Math.min(1, Math.max(0, (x - rect.left) / rect.width)));
    };
    event.preventDefault();
    el.setPointerCapture?.(event.pointerId);
    const [x0, y0] = [event.clientX, event.clientY];
    const from = shownPan();
    let moved = false;
    if (wiping) wipeTo(x0);
    el.classList.add("dragging");

    const move = (e: PointerEvent) => {
      const [dx, dy] = [e.clientX - x0, e.clientY - y0];
      moved ||= Math.hypot(dx, dy) > TAP_SLOP;
      if (wiping) wipeTo(e.clientX);
      else if (zoomed()) setPan(clampPan({ x: from.x + dx, y: from.y + dy }, stage(), shown()));
    };
    const up = () => {
      el.classList.remove("dragging");
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      if (moved) return;
      const now = performance.now();
      const again =
        lastTap !== null &&
        now - lastTap.time < DOUBLE_TAP_MS &&
        Math.hypot(x0 - lastTap.x, y0 - lastTap.y) < TAP_SLOP * 2;
      lastTap = again ? null : { time: now, x: x0, y: y0 };
      if (props.mode === "flip") flip();
      if (again) {
        anchor = { x: x0 - rect.left - rect.width / 2, y: y0 - rect.top - rect.height / 2 };
        props.onZoom(props.zoom === "fit" ? 1 : "fit");
      }
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };

  // The divider's own handle drags it even while zoomed in.
  const pressDivider = (event: PointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const handle = event.currentTarget as HTMLElement;
    const stageEl = handle.parentElement;
    if (!stageEl) return;
    handle.setPointerCapture?.(event.pointerId);
    const rect = stageEl.getBoundingClientRect();
    const move = (e: PointerEvent) => {
      if (rect.width > 0) {
        setDivider(Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)));
      }
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
  };

  const dividerKey = (event: KeyboardEvent) => {
    const step = event.shiftKey ? 0.1 : 0.02;
    const keys: Record<string, number> = {
      ArrowLeft: divider() - step,
      ArrowDown: divider() - step,
      ArrowRight: divider() + step,
      ArrowUp: divider() + step,
      Home: 0,
      End: 1,
    };
    const next = keys[event.key];
    if (next === undefined) return;
    event.preventDefault();
    setDivider(Math.min(1, Math.max(0, next)));
  };

  // Scrolling (two fingers on a trackpad) pans a zoomed picture.
  const scroll = (event: WheelEvent) => {
    if (!zoomed() || event.ctrlKey) return;
    event.preventDefault();
    setPan(
      clampPan(
        { x: shownPan().x - event.deltaX, y: shownPan().y - event.deltaY },
        stage(),
        shown(),
      ),
    );
  };

  const flip = () => setShowing((side) => (side === "after" ? "before" : "after"));

  const contentStyle = () => ({
    width: `${shown().width}px`,
    height: `${shown().height}px`,
    transform: `translate(calc(-50% + ${shownPan().x}px), calc(-50% + ${shownPan().y}px))`,
  });

  const half = (side: Side) => (side === "before" ? props.before : props.after);

  const layer = (side: Side, clip?: () => string) => (
    <div class="compare-layer" data-side={side} style={clip ? { "clip-path": clip() } : undefined}>
      <div class="compare-content" style={contentStyle()}>
        <Show when={half(side).state.sample} fallback={<div class="compare-skeleton" />}>
          {(sample) => (
            <img
              class="compare-img"
              classList={{ pixelated: scale() >= 2 }}
              src={sample().url}
              alt={side === "before" ? `Original of ${props.name}` : `${props.name} converted`}
              draggable={false}
              onLoad={(e) => {
                if (side !== "before") return;
                const img = e.currentTarget;
                if (img.naturalWidth > 0) {
                  setNatural({ width: img.naturalWidth, height: img.naturalHeight });
                }
              }}
              onError={() => half(side).onBroken?.()}
            />
          )}
        </Show>
      </div>
    </div>
  );

  const tag = (side: Side, where: "left" | "right") => (
    <span
      class={`compare-label ${where}`}
      data-side={side}
      aria-busy={half(side).state.busy || undefined}
    >
      {half(side).label}
      <Show when={half(side).state.busy}>
        <span class="compare-spinner" role="status" aria-label="Updating" />
      </Show>
    </span>
  );

  const failure = () => props.after.state.error ?? props.before.state.error;

  return (
    <section
      class="compare"
      data-mode={props.mode}
      classList={{ zoomed: zoomed() }}
      aria-label="Before and after"
    >
      <Show
        when={props.mode === "side"}
        fallback={
          <div ref={stageRef(true)} class="compare-stage" onPointerDown={press}>
            <Show
              when={props.mode === "wipe"}
              fallback={
                <>
                  {layer(showing())}
                  <button
                    type="button"
                    class="compare-label left flip-label"
                    data-side={showing()}
                    aria-pressed={showing() === "before"}
                    aria-busy={half(showing()).state.busy || undefined}
                    title="Tap the picture to flip"
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={flip}
                  >
                    <Icon name="compare" size={14} class="compare-icon" />
                    {half(showing()).label}
                    <span class="flip-hint">· Tap to flip</span>
                    <Show when={half(showing()).state.busy}>
                      <span class="compare-spinner" role="status" aria-label="Updating" />
                    </Show>
                  </button>
                </>
              }
            >
              {layer("before")}
              {layer("after", () => `inset(0 0 0 ${divider() * 100}%)`)}
              <div
                class="compare-divider"
                style={{ left: `${divider() * 100}%` }}
                role="slider"
                tabIndex={0}
                aria-label="Divider between original and converted"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(divider() * 100)}
                aria-valuetext={`${Math.round(divider() * 100)}% original`}
                onPointerDown={pressDivider}
                onKeyDown={dividerKey}
              >
                <span class="compare-knob">
                  <Icon name="chevron-left" size={14} />
                  <Icon name="chevron-right" size={14} />
                </span>
              </div>
              {tag("before", "left")}
              {tag("after", "right")}
            </Show>
          </div>
        }
      >
        <div class="compare-pair">
          <div ref={stageRef(true)} class="compare-stage" onPointerDown={press}>
            {layer("before")}
            {tag("before", "left")}
          </div>
          <div ref={stageRef(false)} class="compare-stage" onPointerDown={press}>
            {layer("after")}
            {tag("after", "left")}
          </div>
        </div>
      </Show>
      <Show when={failure()}>
        {(message) => (
          <p class="compare-error" role="alert">
            <Icon name="alert" size={15} class="compare-error-icon" />
            <span>Couldn't make the preview: {message()}</span>
          </p>
        )}
      </Show>
    </section>
  );
};
