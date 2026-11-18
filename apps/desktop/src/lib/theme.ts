/**
 * Light and dark. The colours are CSS tokens (see the top of `styles.css`):
 * the dark ones apply with `data-theme="dark"` on `<html>`, or when the system
 * is dark and `data-theme` isn't `"light"`. "System" leaves the attribute off.
 *
 * Things drawn outside CSS (the waveform, drag images) read the tokens with
 * {@link cssToken} and redraw when {@link isDark} changes.
 */
import { createSignal } from "solid-js";
import type { ThemeChoice } from "./api";

export type { ThemeChoice } from "./api";

export const THEME_CHOICES: readonly { choice: ThemeChoice; label: string }[] = [
  { choice: "system", label: "Same as system" },
  { choice: "light", label: "Light" },
  { choice: "dark", label: "Dark" },
];

/** Where the choice is kept: on its own, so it can be read before anything else loads. */
export const THEME_KEY = "catconvert.theme.v1";

export function themeFrom(raw: unknown): ThemeChoice {
  return raw === "light" || raw === "dark" ? raw : "system";
}

/** The stored choice; "system" when there is none or storage is unavailable. */
export function storedTheme(): ThemeChoice {
  try {
    const raw = localStorage.getItem(THEME_KEY);
    return themeFrom(raw ? JSON.parse(raw) : null);
  } catch {
    return "system";
  }
}

const darkQuery =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

const [systemDark, setSystemDark] = createSignal(darkQuery?.matches ?? false);
darkQuery?.addEventListener("change", (event) => setSystemDark(event.matches));

const [choice, setChoice] = createSignal<ThemeChoice>("system");

/** Whether the dark theme shows right now, following the system when the choice is "system". */
export function isDark(): boolean {
  const picked = choice();
  return picked === "dark" || (picked === "system" && systemDark());
}

/** Show `theme` in the page: sets or clears `data-theme` on `<html>`. */
export function applyTheme(theme: ThemeChoice) {
  // The attribute first, so whatever redraws on the change reads the new tokens.
  if (typeof document !== "undefined") {
    const root = document.documentElement;
    if (theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", theme);
  }
  setChoice(theme);
}

/** A colour token's current value, e.g. `cssToken("--accent", "#c93b63")`. */
export function cssToken(name: string, fallback: string): string {
  if (typeof document === "undefined") return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

/** A token as `#rrggbb`, for ffmpeg; `fallback` when it is written another way. */
export function hexToken(name: string, fallback: string): string {
  const value = cssToken(name, fallback);
  return /^#[0-9a-f]{6}$/i.test(value) ? value : fallback;
}
