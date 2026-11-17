import { type Component, type JSX, Show } from "solid-js";
import { formatClock } from "../lib/format";

type Props = {
  duration: number;
  start: number;
  end: number;
  /** Time shown in the preview or being played; a line that can be dragged. */
  playhead: number | null;
  /** Playing: the playhead follows the sound without easing. */
  playing?: boolean;
  onChange: (start: number, end: number) => void;
  /** A position worth previewing (handle or range moved). */
  onScrub: (time: number) => void;
  /** Go to a time (track clicked or dragged, playhead moved); defaults to `onScrub`. */
  onSeek?: (time: number) => void;
  /** Fade lengths in seconds, drawn as ramps at the ends of the kept range. */
  fades?: { fadeIn: number; fadeOut: number } | undefined;
  /** Track background: filmstrip or waveform. */
  children?: JSX.Element;
};

type Drag = "start" | "end" | "range";

/** Pixels the range must move before a press on it becomes a drag rather than a click. */
const CLICK_SLOP = 3;

/**
 * Two draggable handles over a filmstrip/waveform. Handles are sliders for
 * keyboard users: arrows nudge 0.1 s, Shift+arrows 1 s. Clicking or dragging
 * the track seeks, as does the playhead (a slider too: arrows 1 s, Shift 5 s).
 */
export const TrimBar: Component<Props> = (props) => {
  let track: HTMLDivElement | undefined;
  const minGap = () => Math.min(0.1, props.duration / 2);
  const pct = (t: number) => (props.duration > 0 ? (t / props.duration) * 100 : 0);
  /** A span as a share of the kept range, in percent. */
  const rampPct = (secs: number) => {
    const length = props.end - props.start;
    return length > 0 ? Math.min(100, (secs / length) * 100) : 0;
  };

  const timeAt = (clientX: number) => {
    if (!track) return 0;
    const rect = track.getBoundingClientRect();
    const ratio = rect.width > 0 ? (clientX - rect.left) / rect.width : 0;
    return Math.min(props.duration, Math.max(0, ratio * props.duration));
  };

  const seek = (t: number) => (props.onSeek ?? props.onScrub)(t);

  /** Follow the pointer with the playhead until the button is let go. */
  const beginSeek = (event: PointerEvent) => {
    if (event.button !== 0 || !track) return;
    event.preventDefault();
    event.stopPropagation();
    const target = track;
    target.setPointerCapture(event.pointerId);
    target.classList.add("seeking");
    seek(timeAt(event.clientX));
    const move = (e: PointerEvent) => seek(timeAt(e.clientX));
    const up = () => {
      target.classList.remove("seeking");
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", up);
  };

  const setStart = (t: number) => {
    const start = Math.min(Math.max(0, t), props.end - minGap());
    props.onChange(start, props.end);
    props.onScrub(start);
  };
  const setEnd = (t: number) => {
    const end = Math.max(Math.min(props.duration, t), props.start + minGap());
    props.onChange(props.start, end);
    props.onScrub(end);
  };

  const beginDrag = (kind: Drag, event: PointerEvent) => {
    // A range spanning the whole file can't move: dragging it seeks.
    if (kind === "range" && props.end - props.start >= props.duration - minGap()) {
      beginSeek(event);
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    target.focus();
    const grabX = event.clientX;
    const grabTime = timeAt(event.clientX);
    const grabStart = props.start;
    const length = props.end - props.start;
    // A press on the range that doesn't move is a click: seek there.
    let moved = kind !== "range";
    track?.classList.add("dragging");

    const move = (e: PointerEvent) => {
      if (!moved && Math.abs(e.clientX - grabX) < CLICK_SLOP) return;
      moved = true;
      const t = timeAt(e.clientX);
      if (kind === "start") setStart(t);
      else if (kind === "end") setEnd(t);
      else {
        const start = Math.min(Math.max(0, grabStart + (t - grabTime)), props.duration - length);
        props.onChange(start, start + length);
        props.onScrub(start);
      }
    };
    const up = (e: PointerEvent) => {
      track?.classList.remove("dragging");
      if (!moved && e.type === "pointerup") seek(timeAt(e.clientX));
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", up);
  };

  const onKey = (kind: "start" | "end", event: KeyboardEvent) => {
    const step = event.shiftKey ? 1 : 0.1;
    const value = kind === "start" ? props.start : props.end;
    const set = kind === "start" ? setStart : setEnd;
    switch (event.key) {
      case "ArrowLeft":
      case "ArrowDown":
        set(value - step);
        break;
      case "ArrowRight":
      case "ArrowUp":
        set(value + step);
        break;
      case "Home":
        set(kind === "start" ? 0 : props.start + minGap());
        break;
      case "End":
        set(kind === "start" ? props.end - minGap() : props.duration);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const onPlayheadKey = (event: KeyboardEvent) => {
    const step = event.shiftKey ? 5 : 1;
    const at = props.playhead ?? 0;
    switch (event.key) {
      case "ArrowLeft":
      case "ArrowDown":
        seek(Math.max(0, at - step));
        break;
      case "ArrowRight":
      case "ArrowUp":
        seek(Math.min(props.duration, at + step));
        break;
      case "Home":
        seek(0);
        break;
      case "End":
        seek(props.duration);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const handle = (kind: "start" | "end") => (
    <div
      role="slider"
      tabIndex={0}
      class={`trim-handle ${kind}`}
      style={{ left: `${pct(kind === "start" ? props.start : props.end)}%` }}
      aria-label={kind === "start" ? "Trim start" : "Trim end"}
      aria-valuemin={kind === "start" ? 0 : props.start}
      aria-valuemax={kind === "start" ? props.end : props.duration}
      aria-valuenow={kind === "start" ? props.start : props.end}
      aria-valuetext={formatClock(kind === "start" ? props.start : props.end, true)}
      onPointerDown={(e) => beginDrag(kind, e)}
      onKeyDown={(e) => onKey(kind, e)}
    >
      <span class="trim-grip" />
    </div>
  );

  return (
    <div
      class="trim-track"
      classList={{ playing: props.playing === true }}
      ref={track}
      onPointerDown={beginSeek}
    >
      <div class="trim-bg">{props.children}</div>
      <div class="trim-shade left" style={{ left: 0, width: `${pct(props.start)}%` }} />
      <div class="trim-shade right" style={{ right: 0, width: `${100 - pct(props.end)}%` }} />
      <Show when={props.fades && (props.fades.fadeIn > 0 || props.fades.fadeOut > 0)}>
        <FadeRamps
          left={pct(props.start)}
          right={100 - pct(props.end)}
          fadeIn={rampPct(props.fades?.fadeIn ?? 0)}
          fadeOut={rampPct(props.fades?.fadeOut ?? 0)}
        />
      </Show>
      <div
        class="trim-range"
        style={{ left: `${pct(props.start)}%`, right: `${100 - pct(props.end)}%` }}
        onPointerDown={(e) => beginDrag("range", e)}
      />
      <Show when={props.playhead !== null}>
        <div
          role="slider"
          tabIndex={0}
          class="trim-playhead"
          style={{ left: `${pct(props.playhead ?? 0)}%` }}
          aria-label="Playback position"
          aria-valuemin={0}
          aria-valuemax={props.duration}
          aria-valuenow={props.playhead ?? 0}
          aria-valuetext={formatClock(props.playhead ?? 0, true)}
          onPointerDown={(e) => {
            e.currentTarget.focus();
            beginSeek(e);
          }}
          onKeyDown={onPlayheadKey}
        >
          <span class="trim-playhead-knob" />
        </div>
      </Show>
      {handle("start")}
      {handle("end")}
    </div>
  );
};

/**
 * The volume envelope over the kept range: the area above each ramp is
 * dimmed, like the trimmed-off ends, so the fade reads as sound going away.
 */
const FadeRamps: Component<{ left: number; right: number; fadeIn: number; fadeOut: number }> = (
  props,
) => (
  <svg
    class="trim-fades"
    style={{ left: `${props.left}%`, right: `${props.right}%` }}
    viewBox="0 0 100 100"
    preserveAspectRatio="none"
    aria-hidden="true"
  >
    <Show when={props.fadeIn > 0}>
      <polygon class="fade-shade" points={`0,0 ${props.fadeIn},0 0,100`} />
      <line class="fade-line" x1="0" y1="100" x2={props.fadeIn} y2="0" />
    </Show>
    <Show when={props.fadeOut > 0}>
      <polygon class="fade-shade" points={`${100 - props.fadeOut},0 100,0 100,100`} />
      <line class="fade-line" x1={100 - props.fadeOut} y1="0" x2="100" y2="100" />
    </Show>
  </svg>
);
