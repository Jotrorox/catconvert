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
      <path d="M27 52 31 13l27 19Z" fill="var(--cat-fur)" />
      <path d="m33.5 40 1.5-18 15 10.5Z" fill="var(--cat-pink)" stroke="none" />
    </g>
    <g class="cat-ear right">
      <path d="M93 52 89 13 62 32Z" fill="var(--cat-fur)" />
      <path d="m86.5 40-1.5-18-15 10.5Z" fill="var(--cat-pink)" stroke="none" />
    </g>
    <path d="M20 94V65c0-26 18-38 40-38s40 12 40 38v29Z" fill="var(--cat-fur)" />
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
      <rect x="6" y="86" width="108" height="18" rx="9" fill="var(--cat-fur)" />
    </Show>
    <ellipse
      class="cat-paw left"
      cx="45"
      cy={props.sleepy ? 87 : 94}
      rx="12"
      ry="7"
      fill="var(--cat-fur)"
    />
    <ellipse
      class="cat-paw right"
      cx="75"
      cy={props.sleepy ? 87 : 94}
      rx="12"
      ry="7"
      fill="var(--cat-fur)"
    />
  </svg>
);
