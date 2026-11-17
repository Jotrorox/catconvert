import { describe, expect, it } from "vitest";
import { parseMbps } from "./QualityField";

describe("parseMbps", () => {
  it("turns what was typed into kbit/s within range", () => {
    expect(parseMbps("4")).toBe(4000);
    expect(parseMbps(" 2.5 ")).toBe(2500);
    expect(parseMbps("0,8")).toBe(800);
    expect(parseMbps("0.01")).toBe(100);
    expect(parseMbps("900")).toBe(200_000);
    expect(parseMbps("")).toBeNull();
    expect(parseMbps("fast")).toBeNull();
  });
});
