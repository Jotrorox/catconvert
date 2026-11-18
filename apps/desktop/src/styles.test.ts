// @vitest-environment node
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(fileURLToPath(new URL("./styles.css", import.meta.url)), "utf8");

const END_OF_TOKENS = "/* End of the colour tokens. */";

/** The declarations inside the first `{ … }` after `selector`, without nested blocks. */
function block(selector: string): string {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`no ${selector} block`);
  let depth = 0;
  for (let i = css.indexOf("{", start); i < css.length; i++) {
    if (css[i] === "{") depth++;
    if (css[i] === "}" && --depth === 0) return css.slice(css.indexOf("{", start) + 1, i);
  }
  throw new Error(`${selector} block never ends`);
}

function tokens(declarations: string): Map<string, string> {
  const found = new Map<string, string>();
  const withoutComments = declarations.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const [, name, value] of withoutComments.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
    found.set(name as string, (value as string).trim().replace(/\s+/g, " "));
  }
  return found;
}

const light = tokens(block(":root"));
const dark = tokens(block(':root[data-theme="dark"]'));
const systemDark = tokens(block(':root:not([data-theme="light"])'));

function luminance(hex: string): number {
  const channel = (i: number) => {
    const c = Number.parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrast(theme: Map<string, string>, fg: string, bg: string): number {
  const colour = (name: string) => {
    const value = theme.get(name);
    if (!value || !/^#[0-9a-f]{6}$/i.test(value)) throw new Error(`${name} isn't #rrggbb`);
    return luminance(value);
  };
  const [hi, lo] = [colour(fg), colour(bg)].sort((a, b) => b - a) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

describe("styles.css", () => {
  it("keeps colours in the token blocks", () => {
    const rest = css.slice(css.indexOf(END_OF_TOKENS));
    expect(css.includes(END_OF_TOKENS)).toBe(true);
    const offenders = rest
      .split("\n")
      .filter((line) => /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(/i.test(line))
      .map((line) => line.trim());
    expect(offenders).toEqual([]);
  });

  it("has the same dark theme when picked and when following the system", () => {
    expect([...systemDark]).toEqual([...dark]);
  });

  it("gives every colour a dark value", () => {
    const missing = [...dark.keys()].filter((name) => !light.has(name));
    expect(missing).toEqual([]);
    const colours = [...light].filter(([, value]) => /#|rgb\(|url\(/.test(value));
    expect(colours.filter(([name]) => !dark.has(name)).map(([name]) => name)).toEqual([]);
  });

  describe.each([
    ["light", light],
    ["dark", dark],
  ])("the %s theme", (_name, theme) => {
    it.each([
      ["--ink", "--bg"],
      ["--ink", "--surface"],
      ["--ink", "--sunken"],
      ["--ink", "--sidebar"],
      ["--muted", "--bg"],
      ["--muted", "--surface"],
      ["--muted", "--sunken"],
      ["--muted", "--sidebar"],
      ["--accent", "--bg"],
      ["--accent", "--surface"],
      ["--accent-strong", "--accent-soft"],
      ["--on-accent", "--accent"],
      ["--on-accent", "--accent-hover"],
      ["--toast-ink", "--toast-bg"],
      ...["pink", "green", "yellow", "lavender"].flatMap((tone) => [
        [`--${tone}-ink`, `--${tone}-bg`],
        [`--${tone}-ink`, "--surface"],
        // A picked chip: the tag's ink as background.
        ["--on-accent", `--${tone}-ink`],
      ]),
    ])("reads %s on %s (AA)", (fg, bg) => {
      expect(contrast(theme, fg, bg)).toBeGreaterThanOrEqual(4.5);
    });

    it("shows focus rings (3:1)", () => {
      expect(contrast(theme, "--accent", "--bg")).toBeGreaterThanOrEqual(3);
      expect(contrast(theme, "--accent", "--surface")).toBeGreaterThanOrEqual(3);
    });
  });
});
