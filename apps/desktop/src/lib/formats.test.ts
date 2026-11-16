import { describe, expect, it } from "vitest";
import type { MediaInfo } from "./api";
import { defaultFormat, fpsChoices, initialOptions, RECIPES, sizeChoices } from "./formats";

const clip: MediaInfo = {
  path: "/a/clip.mov",
  fileName: "clip.mov",
  extension: "mov",
  sizeBytes: 1,
  kind: "video",
  durationSecs: 10,
  width: 640,
  height: 360,
  fps: 24,
  videoCodec: "h264",
  audioCodec: "aac",
};

const all = () => true;

describe("formats", () => {
  it("only offers downscales and slower frame rates", () => {
    expect(sizeChoices(clip)).toEqual([240]);
    expect(fpsChoices(clip)).toEqual([15, 10]);
  });

  it("never defaults to the input's own format", () => {
    expect(defaultFormat({ ...clip, extension: "mp4" }, all)).toBe("webm");
    expect(defaultFormat({ ...clip, kind: "image", extension: "png" }, all)).toBe("jpg");
    expect(defaultFormat({ ...clip, kind: "audio", extension: "mp3" }, all)).toBe("m4a");
    expect(defaultFormat(clip, (f) => f !== "mp4")).toBe("webm");
  });

  it("applies recipes but drops settings that would upscale", () => {
    const gif = RECIPES.find((r) => r.id === "gif") ?? null;
    const options = initialOptions(clip, all, gif);
    expect(options.format).toBe("gif");
    expect(options.maxSize).toBeNull(); // 480 > 360
    expect(options.fps).toBe(15);

    const forImage = initialOptions({ ...clip, kind: "image" }, all, gif);
    expect(forImage.format).toBe("png");
  });
});
