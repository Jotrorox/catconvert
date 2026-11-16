import { type Component, For, Show } from "solid-js";
import { CatPeek } from "../components/Cat";
import { clearHistory, state } from "../lib/store";
import { RecentRow } from "./ConvertHome";

export const HistoryScreen: Component = () => (
  <div class="page">
    <header class="page-header">
      <div>
        <h1 class="display page-title">History</h1>
        <p class="subtitle">Everything the kitty has kneaded on this computer.</p>
      </div>
      <Show when={state.history.length > 0}>
        <button type="button" class="btn btn-outline" onClick={() => clearHistory()}>
          Forget all
        </button>
      </Show>
    </header>
    <Show
      when={state.history.length > 0}
      fallback={
        <section class="card placeholder">
          <CatPeek sleepy class="placeholder-cat" />
          <h2 class="label-strong">A clean slate</h2>
          <p class="item-meta">Finished conversions show up here.</p>
        </section>
      }
    >
      <section class="card recent">
        <ul class="recent-list">
          <For each={state.history}>{(item) => <RecentRow item={item} />}</For>
        </ul>
      </section>
    </Show>
  </div>
);
