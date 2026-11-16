import { type Accessor, createSignal, onCleanup } from "solid-js";

/** Reactive `matchMedia`. Falls back to `false` where it is unavailable (e.g. jsdom). */
export function createMediaQuery(query: string): Accessor<boolean> {
  const mql = typeof window.matchMedia === "function" ? window.matchMedia(query) : null;
  const [matches, setMatches] = createSignal(mql?.matches ?? false);
  if (mql) {
    const onChange = (event: MediaQueryListEvent) => setMatches(event.matches);
    mql.addEventListener("change", onChange);
    onCleanup(() => mql.removeEventListener("change", onChange));
  }
  return matches;
}
