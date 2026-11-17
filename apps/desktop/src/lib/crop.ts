/**
 * Crop geometry for the editor. A crop is measured on the picture as it looks
 * after the turn and mirror, in fractions of its width and height, so it
 * doesn't depend on the preview's size. `crop_box` in convert.rs turns it
 * into pixels; `cropBox` here mirrors it so the UI shows the same result.
 */
import type { Aspect, ConvertOptions, Crop, CropArea, MediaInfo } from "./api";

export const ASPECTS: readonly { label: string; aspect: Aspect | null }[] = [
  { label: "Free", aspect: null },
  { label: "1:1", aspect: [1, 1] },
  { label: "9:16", aspect: [9, 16] },
  { label: "16:9", aspect: [16, 9] },
];

export type Size = { width: number; height: number };
export type Box = Size & { x: number; y: number };

const FULL: CropArea = { x: 0, y: 0, width: 1, height: 1 };

/** Smallest crop side, as a share of the picture's short side. */
const MIN_SIDE = 0.06;

export const aspectText = (aspect: Aspect) => `${aspect[0]}:${aspect[1]}`;

export function sameAspect(a: Aspect | null, b: Aspect | null): boolean {
  return a === b || (a !== null && b !== null && a[0] * b[1] === a[1] * b[0]);
}

/** The picture's size after the turn, if known. */
export function turnedSize(info: MediaInfo, rotate: ConvertOptions["rotate"]): Size | null {
  if (!info.width || !info.height) return null;
  return rotate % 180 === 90
    ? { width: info.height, height: info.width }
    : { width: info.width, height: info.height };
}

/** The crop in pixels, or `null` when it keeps the whole picture. */
export function cropBox(size: Size, crop: Crop | null): Box | null {
  if (!crop) return null;
  const { width: w, height: h } = size;
  const area = crop.area ?? FULL;
  const px = (v: number, max: number) => Math.min(max, Math.max(1, Math.round(v)));
  let width = px(area.width * w, w);
  let height = px(area.height * h, h);
  if (crop.aspect) {
    const [aw, ah] = crop.aspect;
    if (width * ah <= height * aw) height = px((width * ah) / aw, height);
    else width = px((height * aw) / ah, width);
  }
  if (width === w && height === h) return null;
  const place = (start: number, span: number, kept: number, total: number) =>
    Math.min(total - kept, Math.max(0, Math.round((start + span / 2) * total - kept / 2)));
  return {
    width,
    height,
    x: place(area.x, area.width, width, w),
    y: place(area.y, area.height, height, h),
  };
}

/** The area a crop keeps, in fractions; the whole picture when there is none. */
export function keptArea(size: Size, crop: Crop | null): CropArea {
  const box = cropBox(size, crop);
  if (!box) return FULL;
  return {
    x: box.x / size.width,
    y: box.y / size.height,
    width: box.width / size.width,
    height: box.height / size.height,
  };
}

/** The picture's size after the turn and crop, if known. */
export function pictureSize(
  info: MediaInfo,
  options: Pick<ConvertOptions, "rotate" | "crop">,
): Size | null {
  const size = turnedSize(info, options.rotate);
  if (!size) return null;
  const box = cropBox(size, options.crop);
  return box ? { width: box.width, height: box.height } : size;
}

/** The same crop after turning the picture 90° clockwise: it stays on the same pixels. */
export function turnCrop(crop: Crop): Crop {
  const a = crop.area;
  return {
    aspect: crop.aspect && [crop.aspect[1], crop.aspect[0]],
    area: a && tidy({ x: 1 - a.y - a.height, y: a.x, width: a.height, height: a.width }),
  };
}

/** The same crop after mirroring the picture. */
export function mirrorCrop(crop: Crop): Crop {
  const a = crop.area;
  return { ...crop, area: a && tidy({ ...a, x: 1 - a.x - a.width }) };
}

/**
 * The crop for another file: a shape is kept and centred, a hand-drawn area
 * belongs to the picture it was drawn on.
 */
export function cropForOtherFile(crop: Crop | null): Crop | null {
  return crop?.aspect ? { aspect: [...crop.aspect], area: null } : null;
}

/** Round to 4 decimals and keep the area inside the picture. */
function tidy(area: CropArea): CropArea {
  const r = (v: number) => Math.round(v * 10_000) / 10_000;
  const x = Math.min(1, Math.max(0, r(area.x)));
  const y = Math.min(1, Math.max(0, r(area.y)));
  return {
    x,
    y,
    width: Math.min(1 - x, Math.max(0, r(area.width))),
    height: Math.min(1 - y, Math.max(0, r(area.height))),
  };
}

/** `area` shifted by `dx`, `dy` (fractions), stopping at the picture's edges. */
export function moveArea(area: CropArea, dx: number, dy: number): CropArea {
  return tidy({
    ...area,
    x: Math.min(1 - area.width, Math.max(0, area.x + dx)),
    y: Math.min(1 - area.height, Math.max(0, area.y + dy)),
  });
}

export type Corner = "nw" | "ne" | "sw" | "se";

/**
 * `area` with `corner` dragged to `point` (fractions) while the opposite
 * corner stays put. With an aspect the box keeps its shape and grows to
 * whichever side the pointer pulls further, without leaving the picture.
 */
export function resizeArea(
  size: Size,
  aspect: Aspect | null,
  area: CropArea,
  corner: Corner,
  point: { x: number; y: number },
): CropArea {
  const { width: W, height: H } = size;
  const east = corner.endsWith("e");
  const south = corner.startsWith("s");
  // Work in pixels so the aspect is a plain ratio.
  const ax = (east ? area.x : area.x + area.width) * W;
  const ay = (south ? area.y : area.y + area.height) * H;
  const px = Math.min(W, Math.max(0, point.x * W));
  const py = Math.min(H, Math.max(0, point.y * H));
  const roomX = east ? W - ax : ax;
  const roomY = south ? H - ay : ay;
  const min = Math.min(W, H) * MIN_SIDE;
  const dx = east ? px - ax : ax - px;
  const dy = south ? py - ay : ay - py;

  let w: number;
  let h: number;
  if (aspect) {
    const ratio = aspect[0] / aspect[1];
    w = Math.max(dx, dy * ratio, min * Math.max(1, ratio));
    w = Math.min(w, roomX, roomY * ratio);
    h = w / ratio;
  } else {
    w = Math.min(roomX, Math.max(min, dx));
    h = Math.min(roomY, Math.max(min, dy));
  }
  return tidy({
    x: (east ? ax : ax - w) / W,
    y: (south ? ay : ay - h) / H,
    width: w / W,
    height: h / H,
  });
}

/** Short words for the changes list and recipe cards. */
export function cropLabel(size: Size | null, crop: Crop | null): string | null {
  if (!crop || (size && !cropBox(size, crop))) return null;
  if (crop.aspect) return `Cropped ${aspectText(crop.aspect)}`;
  const box = size && cropBox(size, crop);
  return box ? `Cropped ${box.width}×${box.height}` : "Cropped";
}
