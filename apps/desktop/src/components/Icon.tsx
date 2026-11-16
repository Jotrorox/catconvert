import type { Component, JSX } from "solid-js";

const ICONS = {
  convert: () => <path d="M4 8h15M15 4l4 4-4 4M20 16H5M9 12l-4 4 4 4" />,
  queue: () => <path d="M4 6h16M4 12h16M4 18h10" />,
  star: () => (
    <path d="m12 3.5 2.6 5.3 5.9.9-4.25 4.1 1 5.8L12 16.85 6.75 19.6l1-5.8L3.5 9.7l5.9-.9Z" />
  ),
  clock: () => (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 2" />
    </>
  ),
  sliders: () => (
    <>
      <path d="M4 8h9M19 8h1M4 16h3M13 16h7" />
      <circle cx="16" cy="8" r="2.5" />
      <circle cx="10" cy="16" r="2.5" />
    </>
  ),
  plus: () => <path d="M12 5v14M5 12h14" />,
  "chevron-left": () => <path d="m14.5 6-6 6 6 6" />,
  "chevron-right": () => <path d="m9.5 6 6 6-6 6" />,
  video: () => (
    <>
      <rect x="3" y="6.5" width="12" height="11" rx="2.5" />
      <path d="m15 10.5 5.5-3v9l-5.5-3" />
    </>
  ),
  music: () => (
    <>
      <path d="M9 17.5V6l10-2v11.5" />
      <circle cx="6.5" cy="17.5" r="2.5" />
      <circle cx="16.5" cy="15.5" r="2.5" />
    </>
  ),
  check: () => <path d="m5 12.5 4.5 4.5L19 7.5" />,
  close: () => <path d="M6.5 6.5l11 11M17.5 6.5l-11 11" />,
  pause: () => (
    <g fill="currentColor" stroke="none">
      <rect x="6.5" y="5" width="3.6" height="14" rx="1.4" />
      <rect x="13.9" y="5" width="3.6" height="14" rx="1.4" />
    </g>
  ),
  play: () => <path d="M8.5 5.8v12.4L18.5 12Z" fill="currentColor" />,
  paw: () => (
    <g fill="currentColor" stroke="none">
      <ellipse cx="12" cy="16" rx="4.8" ry="3.9" />
      <ellipse cx="5.6" cy="10.6" rx="2.1" ry="2.6" />
      <ellipse cx="9.6" cy="6.6" rx="2.1" ry="2.7" />
      <ellipse cx="14.4" cy="6.6" rx="2.1" ry="2.7" />
      <ellipse cx="18.4" cy="10.6" rx="2.1" ry="2.6" />
    </g>
  ),
  scissors: () => (
    <>
      <circle cx="6.5" cy="6.5" r="2.8" />
      <circle cx="6.5" cy="17.5" r="2.8" />
      <path d="M8.8 8.2 20 18M8.8 15.8 20 6" />
    </>
  ),
  share: () => <path d="M12 15V3.5M8 7.5l4-4 4 4M5.5 12v6.5a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2V12" />,
  rotate: () => (
    <>
      <path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" />
      <path d="M19.5 4.5v4h-4" />
    </>
  ),
  flip: () => (
    <>
      <path d="M12 3.5v17" stroke-dasharray="2 2.6" />
      <path d="M9 7 3.5 17H9ZM15 7l5.5 10H15Z" />
    </>
  ),
  folder: () => (
    <path d="M3.5 7.5a2 2 0 0 1 2-2h3.8l2 2.2h7.2a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2Z" />
  ),
  image: () => (
    <>
      <rect x="3.5" y="5" width="17" height="14" rx="3" />
      <circle cx="9" cy="10" r="1.6" />
      <path d="m4.5 17.5 5-4.5 3.5 3 2.5-2 4 3.5" />
    </>
  ),
  sparkle: () => (
    <path d="M12 3.5c.6 4.2 2.3 5.9 6.5 6.5-4.2.6-5.9 2.3-6.5 6.5-.6-4.2-2.3-5.9-6.5-6.5 4.2-.6 5.9-2.3 6.5-6.5ZM18.5 15.5c.3 1.8 1 2.5 2.5 2.8-1.5.3-2.2 1-2.5 2.7-.3-1.7-1-2.4-2.5-2.7 1.5-.3 2.2-1 2.5-2.8Z" />
  ),
  refresh: () => (
    <>
      <path d="M4.5 12a7.5 7.5 0 0 1 13-5.1L19.5 9" />
      <path d="M19.5 4.5V9H15M19.5 12a7.5 7.5 0 0 1-13 5.1L4.5 15" />
      <path d="M4.5 19.5V15H9" />
    </>
  ),
  "arrow-right": () => <path d="M4.5 12h15M13.5 6l6 6-6 6" />,
  skip: () => <path d="m6 6 7 6-7 6ZM17.5 6v12" />,
  alert: () => (
    <>
      <path d="M12 4 21 19.5H3Z" />
      <path d="M12 10v4M12 17v.01" />
    </>
  ),
  terminal: () => (
    <>
      <rect x="3" y="4.5" width="18" height="15" rx="3" />
      <path d="m7 9.5 3 2.5-3 2.5M12.5 15H17" />
    </>
  ),
} satisfies Record<string, () => JSX.Element>;

export type IconName = keyof typeof ICONS;

type Props = {
  name: IconName;
  size?: number;
  class?: string;
};

/** Inline stroke icon that inherits `currentColor`. */
export const Icon: Component<Props> = (props) => (
  <svg
    class={props.class ? `icon ${props.class}` : "icon"}
    width={props.size ?? 20}
    height={props.size ?? 20}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.9"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {ICONS[props.name]()}
  </svg>
);
