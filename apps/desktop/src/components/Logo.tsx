import type { Component } from "solid-js";

/**
 * The CatConvert mark: a black C with cat ears next to an orange C. Same
 * shapes as assets/logo.svg at the repo root, cropped to the artwork.
 */
export const Logo: Component<{ class?: string }> = (props) => (
  <svg class={props.class} viewBox="20 30 228 184" aria-hidden="true">
    <path
      fill="#1d1a1b"
      d="M130.42 91.44A66 66 0 1 0 130.42 192.56A16.5 16.5 0 0 0 109.21 167.28A33 33 0 1 1 109.21 116.72A16.5 16.5 0 0 0 130.42 91.44Z"
    />
    <path
      fill="#1d1a1b"
      stroke="#1d1a1b"
      stroke-width="8"
      stroke-linejoin="round"
      d="M50 96L50 36L80 78ZM96 78L126 36L126 96Z"
    />
    <path
      fill="#f9a24f"
      d="M227.46 124.84A50 50 0 1 0 227.46 199.16A13 13 0 0 0 210.06 179.84A24 24 0 1 1 210.06 144.16A13 13 0 0 0 227.46 124.84Z"
    />
  </svg>
);
