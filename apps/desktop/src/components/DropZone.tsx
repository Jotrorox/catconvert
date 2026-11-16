import type { Component } from "solid-js";

type Props = {
  hovering: boolean;
};

/** Visual target for native file drops. The drop itself is handled window-wide via Tauri. */
export const DropZone: Component<Props> = (props) => (
  <section class="drop-zone" classList={{ hovering: props.hovering }} aria-label="Drop zone">
    <p class="drop-title">
      {props.hovering ? "Release to add files" : "Drop photos, videos or audio here"}
    </p>
    <p class="drop-hint">Files stay on your device. Nothing is uploaded.</p>
  </section>
);
