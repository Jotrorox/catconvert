/**
 * Dragging finished files out of the window, into Finder, Explorer, a chat or
 * an email. The drag itself is native (see `dragFiles` in `./api`); this
 * module turns a press-and-move into one and draws the image under the pointer.
 */
import type { MediaKind, OutputFormat } from "./api";
import { KIND_TONE, label, type Tone } from "./formats";
import { cssToken } from "./theme";

/** How far the pointer moves with the button down before a press becomes a drag. */
const DRAG_DISTANCE = 6;

/**
 * Call from `pointerdown`: runs `start` once the pointer has moved a few
 * pixels with the button held. Presses on buttons and links stay clicks.
 */
export function startDragOnMove(event: PointerEvent, start: () => void) {
  if (event.button !== 0) return;
  if (!(event.target instanceof Element) || event.target.closest("button, a, input, select")) {
    return;
  }
  const x = event.clientX;
  const y = event.clientY;
  const move = (e: PointerEvent) => {
    if (Math.hypot(e.clientX - x, e.clientY - y) < DRAG_DISTANCE) return;
    stop();
    start();
  };
  const stop = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", stop);
    window.removeEventListener("pointercancel", stop);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", stop);
  window.addEventListener("pointercancel", stop);
}

/** Drag image size. macOS shows a PNG's pixels as points. */
const ICON_SIZE = 64;

/**
 * A tile with the format's name ("MP4") in the kind's colours, as a PNG data
 * URL for the drag image. Empty if the webview can't draw one.
 */
export function dragIcon(format: OutputFormat, kind: MediaKind): string {
  const canvas = document.createElement("canvas");
  canvas.width = ICON_SIZE;
  canvas.height = ICON_SIZE;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";

  const tone: Tone = KIND_TONE[kind];
  const inset = 6;
  const side = ICON_SIZE - inset * 2;

  // The current theme's colours; the fallbacks are the light theme's.
  ctx.shadowColor = cssToken("--tile-shadow", "rgb(90 30 45 / 0.3)");
  ctx.shadowBlur = 4;
  ctx.shadowOffsetY = 1;
  ctx.fillStyle = cssToken(`--${tone}-bg`, "#fde4e8");
  ctx.beginPath();
  // roundRect is missing from older WebKit; a square tile still works.
  if (typeof ctx.roundRect === "function") ctx.roundRect(inset, inset, side, side, 14);
  else ctx.rect(inset, inset, side, side);
  ctx.fill();

  const text = label(format);
  ctx.shadowColor = "transparent";
  ctx.fillStyle = cssToken(`--${tone}-ink`, "#b3325a");
  ctx.font = `700 ${text.length > 3 ? 13 : 16}px ${cssToken("--font-display", "sans-serif")}`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(text, ICON_SIZE / 2, ICON_SIZE / 2 + 1, side - 8);
  return canvas.toDataURL("image/png");
}
