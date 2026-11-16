/**
 * True when running inside the Tauri shell on macOS, where the window uses an
 * overlay title bar: the traffic lights float over the webview, so the UI has
 * to leave room for them and provide its own drag region.
 */
export const overlayTitleBar =
  typeof window !== "undefined" &&
  "__TAURI_INTERNALS__" in window &&
  /Mac/.test(navigator.userAgent);
