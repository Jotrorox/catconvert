import { describe, expect, it } from "vitest";
import type { Crop } from "./api";
import {
  cropBox,
  cropForOtherFile,
  cropLabel,
  keptArea,
  mirrorCrop,
  moveArea,
  resizeArea,
  sameAspect,
  turnCrop,
} from "./crop";

const WIDE = { width: 1920, height: 1080 };

describe("crop geometry", () => {
  it("works out the same pixels as convert.rs", () => {
    // Same cases as the Rust tests in convert.rs.
    expect(cropBox(WIDE, { aspect: [9, 16], area: null })).toEqual({
      width: 608,
      height: 1080,
      x: 656,
      y: 0,
    });
    expect(
      cropBox(WIDE, { aspect: null, area: { x: 0.1, y: 0.2, width: 0.3001, height: 0.4 } }),
    ).toEqual({ width: 576, height: 432, x: 192, y: 216 });
    // A slightly uneven square area still comes out square, inside the picture.
    expect(
      cropBox(WIDE, { aspect: [1, 1], area: { x: 0.4374, y: 0, width: 0.5626, height: 0.999 } }),
    ).toEqual({ width: 1079, height: 1079, x: 840, y: 0 });
  });

  it("treats a crop of the whole picture as none", () => {
    expect(cropBox(WIDE, null)).toBeNull();
    expect(cropBox(WIDE, { aspect: [16, 9], area: null })).toBeNull();
    expect(cropBox(WIDE, { aspect: null, area: null })).toBeNull();
    expect(keptArea(WIDE, null)).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });

  it("keeps the crop on the same pixels when the picture turns or mirrors", () => {
    const crop: Crop = { aspect: [9, 16], area: { x: 0.1, y: 0.2, width: 0.3, height: 0.5 } };
    expect(turnCrop(crop)).toEqual({
      aspect: [16, 9],
      area: { x: 0.3, y: 0.1, width: 0.5, height: 0.3 },
    });
    expect(turnCrop(turnCrop(turnCrop(turnCrop(crop))))).toEqual(crop);
    expect(mirrorCrop(crop).area).toEqual({ x: 0.6, y: 0.2, width: 0.3, height: 0.5 });
    expect(mirrorCrop(mirrorCrop(crop))).toEqual(crop);
    expect(turnCrop({ aspect: [1, 1], area: null })).toEqual({ aspect: [1, 1], area: null });
  });

  it("moves and resizes within the picture", () => {
    const area = { x: 0.2, y: 0.2, width: 0.5, height: 0.5 };
    expect(moveArea(area, 0.6, -0.3)).toEqual({ x: 0.5, y: 0, width: 0.5, height: 0.5 });

    // Free: the dragged corner follows the pointer, the opposite one stays.
    expect(resizeArea(WIDE, null, area, "se", { x: 0.9, y: 1.4 })).toEqual({
      x: 0.2,
      y: 0.2,
      width: 0.7,
      height: 0.8,
    });
    expect(resizeArea(WIDE, null, area, "nw", { x: 0.5, y: 0.6 })).toEqual({
      x: 0.5,
      y: 0.6,
      width: 0.2,
      height: 0.1,
    });

    // With a shape it stays that shape and stops at the edge.
    const square = { x: 0.25, y: 0.25, width: 0.25, height: 0.25 * (1920 / 1080) };
    const grown = resizeArea(WIDE, [1, 1], square, "se", { x: 1, y: 1 });
    const box = cropBox(WIDE, { aspect: null, area: grown });
    expect(box?.width).toBe(box?.height);
    expect(grown.y + grown.height).toBeLessThanOrEqual(1);
    expect(box?.height).toBe(Math.round(1080 * 0.75));
  });

  it("hands only the shape to other files", () => {
    expect(
      cropForOtherFile({ aspect: [9, 16], area: { x: 0, y: 0, width: 0.3, height: 1 } }),
    ).toEqual({ aspect: [9, 16], area: null });
    expect(
      cropForOtherFile({ aspect: null, area: { x: 0, y: 0, width: 0.3, height: 1 } }),
    ).toBeNull();
    expect(cropForOtherFile(null)).toBeNull();
  });

  it("describes crops and compares shapes", () => {
    expect(cropLabel(WIDE, { aspect: [9, 16], area: null })).toBe("Cropped 9:16");
    expect(cropLabel(WIDE, { aspect: [16, 9], area: null })).toBeNull();
    expect(cropLabel(WIDE, { aspect: null, area: { x: 0, y: 0, width: 0.5, height: 0.5 } })).toBe(
      "Cropped 960×540",
    );
    expect(cropLabel(null, { aspect: null, area: { x: 0, y: 0, width: 0.5, height: 1 } })).toBe(
      "Cropped",
    );
    expect(sameAspect([2, 2], [1, 1])).toBe(true);
    expect(sameAspect([9, 16], [16, 9])).toBe(false);
    expect(sameAspect(null, null)).toBe(true);
  });
});
