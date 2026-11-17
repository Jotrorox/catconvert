import { describe, expect, it } from "vitest";
import {
  dirName,
  extensionNote,
  isPattern,
  isUsual,
  mediaExtension,
  splitChosen,
  targetOf,
  USUAL_OUTPUT,
  withoutExtension,
} from "./output";

describe("output names", () => {
  it("knows the usual place", () => {
    expect(isUsual(USUAL_OUTPUT)).toBe(true);
    expect(isUsual({ folder: null, name: "  " })).toBe(true);
    expect(isUsual({ folder: "/out", name: null })).toBe(false);
    expect(isUsual({ folder: null, name: "holiday" })).toBe(false);
  });

  it("turns a choice into a target", () => {
    expect(targetOf(USUAL_OUTPUT)).toEqual({ folder: null, name: null, collision: "keepBoth" });
    expect(targetOf({ folder: "/out", name: " ", collision: "overwrite" })).toEqual({
      folder: "/out",
      name: null,
      collision: "overwrite",
    });
  });

  it("spots patterns", () => {
    expect(isPattern("{name}-{format}")).toBe(true);
    expect(isPattern("holiday {2026}")).toBe(true);
    expect(isPattern("holiday {}")).toBe(false);
    expect(isPattern(null)).toBe(false);
  });

  it("takes paths apart", () => {
    expect(dirName("/a/b/clip.mp4")).toBe("/a/b");
    expect(dirName("/clip.mp4")).toBe("/");
    expect(dirName("C:\\Videos\\clip.mp4")).toBe("C:\\Videos");
    expect(dirName("C:\\clip.mp4")).toBe("C:\\");
    expect(dirName("clip.mp4")).toBe("");
    expect(splitChosen("/out/Holiday.MOV")).toEqual({ folder: "/out", name: "Holiday" });
    expect(splitChosen("C:\\out\\v1.2.mp4")).toEqual({ folder: "C:\\out", name: "v1.2" });
    expect(splitChosen("/out/notes.txt")).toEqual({ folder: "/out", name: "notes.txt" });
  });

  it("lets the format decide the extension", () => {
    expect(mediaExtension("clip.MOV")).toBe("mov");
    expect(mediaExtension("v1.2")).toBeNull();
    expect(mediaExtension(".mp4")).toBeNull();
    expect(withoutExtension("clip.mp4")).toBe("clip");
    expect(withoutExtension("clip .webm")).toBe("clip");
    expect(withoutExtension("v1.2")).toBe("v1.2");
    expect(extensionNote("clip.mp4", "mp4")).toBeNull();
    expect(extensionNote("cat.jpeg", "jpg")).toBeNull();
    expect(extensionNote("clip", "mp4")).toBeNull();
    expect(extensionNote("clip.mov", "mp4")).toBe(
      "The format decides the extension, so it's saved as .mp4, not .mov.",
    );
  });
});
