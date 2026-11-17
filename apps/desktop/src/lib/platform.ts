/**
 * True when running inside the Tauri shell on macOS, where the window uses an
 * overlay title bar: the traffic lights float over the webview, so the UI has
 * to leave room for them and provide its own drag region.
 */
export const overlayTitleBar =
  typeof window !== "undefined" &&
  "__TAURI_INTERNALS__" in window &&
  /Mac/.test(navigator.userAgent);

const userAgent = typeof navigator === "undefined" ? "" : navigator.userAgent;

/** The desktop OS, from the webview's user agent, for OS-specific wording. */
export const os: "macos" | "windows" | "linux" | "other" = /Mac/.test(userAgent)
  ? "macos"
  : /Windows/.test(userAgent)
    ? "windows"
    : /Linux|X11/.test(userAgent)
      ? "linux"
      : "other";
