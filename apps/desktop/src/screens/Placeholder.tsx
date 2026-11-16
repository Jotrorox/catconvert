import type { Component } from "solid-js";
import { CatPeek } from "../components/Cat";

type Props = {
  title: string;
  message: string;
};

/** Stand-in for screens that have no design yet. */
export const Placeholder: Component<Props> = (props) => (
  <div class="page">
    <header class="page-header">
      <h1 class="display page-title">{props.title}</h1>
    </header>
    <section class="card placeholder">
      <CatPeek sleepy class="placeholder-cat" />
      <h2 class="label-strong">Still napping</h2>
      <p class="item-meta">{props.message}</p>
    </section>
  </div>
);
