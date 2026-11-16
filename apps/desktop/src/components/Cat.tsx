import { type Component, Show } from "solid-js";

type CatProps = {
  class?: string;
  /** Closed, flat eyes and a cushion instead of a table edge. */
  sleepy?: boolean;
  whiskers?: boolean;
};

/**
 * The mascot peeking over an edge, paws first. Ears, eyes and paws carry
 * classes (`cat-ear`, `cat-eyes`, `cat-paw`) so CSS can animate them.
 */
export const CatPeek: Component<CatProps> = (props) => (
  <svg
    class={props.class ? `cat ${props.class}` : "cat"}
    viewBox="0 0 120 106"
    fill="none"
    stroke="currentColor"
    stroke-width="3.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <Show when={!props.sleepy}>
      <ellipse cx="60" cy="97" rx="46" ry="5" fill="var(--cat-shadow)" stroke="none" />
    </Show>
    <g class="cat-ear left">
      <path d="M27 52 31 13l27 19Z" fill="#fff" />
      <path d="m33.5 40 1.5-18 15 10.5Z" fill="var(--cat-pink)" stroke="none" />
    </g>
    <g class="cat-ear right">
      <path d="M93 52 89 13 62 32Z" fill="#fff" />
      <path d="m86.5 40-1.5-18-15 10.5Z" fill="var(--cat-pink)" stroke="none" />
    </g>
    <path d="M20 94V65c0-26 18-38 40-38s40 12 40 38v29Z" fill="#fff" />
    <Show
      when={props.sleepy}
      fallback={
        <path class="cat-eyes" d="M38.5 61q6.5-7 13 0M68.5 61q6.5-7 13 0" stroke-width="3.2" />
      }
    >
      <path d="M38.5 60h13M68.5 60h13" stroke-width="3.2" />
    </Show>
    <ellipse cx="36" cy="70" rx="6.5" ry="3.6" fill="var(--cat-pink)" stroke="none" />
    <ellipse cx="84" cy="70" rx="6.5" ry="3.6" fill="var(--cat-pink)" stroke="none" />
    <path d="M53.5 67.5q3.25 4.5 6.5 0 3.25 4.5 6.5 0" stroke-width="3" />
    <Show when={props.whiskers}>
      <path d="M20 66 5 63.5M20 72.5 5 74.5M100 66l15-2.5M100 72.5l15 2" stroke-width="2.6" />
    </Show>
    <Show when={props.sleepy}>
      <rect x="6" y="86" width="108" height="18" rx="9" fill="#fff" />
    </Show>
    <ellipse class="cat-paw left" cx="45" cy={props.sleepy ? 87 : 94} rx="12" ry="7" fill="#fff" />
    <ellipse class="cat-paw right" cx="75" cy={props.sleepy ? 87 : 94} rx="12" ry="7" fill="#fff" />
  </svg>
);

/** Small round cat face used as the app logo. */
export const CatLogo: Component<{ class?: string }> = (props) => (
  <svg
    class={props.class ? `cat ${props.class}` : "cat"}
    viewBox="0 0 40 38"
    fill="none"
    stroke="currentColor"
    stroke-width="2.4"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path d="M7.5 17 8.5 3.5l9 6.5M32.5 17l-1-13.5-9 6.5" fill="#fff" />
    <path
      d="m10.5 12.5.4-5.5 3.8 2.8ZM29.5 12.5l-.4-5.5-3.8 2.8Z"
      fill="var(--cat-pink)"
      stroke="none"
    />
    <ellipse cx="20" cy="22.5" rx="15" ry="13" fill="#fff" />
    <path d="M12.5 21.5q2.5-2.8 5 0M22.5 21.5q2.5-2.8 5 0" stroke-width="2" />
    <ellipse cx="11.5" cy="26" rx="2.6" ry="1.5" fill="var(--cat-pink)" stroke="none" />
    <ellipse cx="28.5" cy="26" rx="2.6" ry="1.5" fill="var(--cat-pink)" stroke="none" />
    <path d="M17.5 26q1.25 1.8 2.5 0 1.25 1.8 2.5 0" stroke-width="1.8" />
  </svg>
);
