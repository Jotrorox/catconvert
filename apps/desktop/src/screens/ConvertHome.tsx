import { type Component, For, Show } from "solid-js";
import { CatPeek } from "../components/Cat";
import { Icon } from "../components/Icon";
import { Brand } from "../components/Navigation";
import {
  LATEST,
  RECENT,
  RECIPES,
  type RecentItem,
  SUPPORTED_CHIPS,
  type TargetFormat,
} from "../lib/mock";

type Props = {
  compact: boolean;
  onOpenFile: (format?: TargetFormat) => void;
  onOpenSettings: () => void;
};

const RecentIcon: Component<{ item: RecentItem }> = (props) => (
  <span class="tile" data-tone={props.item.kind === "video" ? "pink" : "yellow"}>
    <Icon name={props.item.kind === "video" ? "video" : "music"} size={18} />
  </span>
);

export const ConvertHome: Component<Props> = (props) => (
  <div class="page">
    <Show
      when={props.compact}
      fallback={
        <header class="page-header">
          <div>
            <h1 class="display page-title">Convert</h1>
            <p class="subtitle">Drop a file and we'll knead it into shape.</p>
          </div>
          <button type="button" class="btn btn-outline">
            <Icon name="plus" size={18} />
            Add folder
          </button>
        </header>
      }
    >
      <header class="mobile-top">
        <Brand />
        <button
          type="button"
          class="icon-btn round"
          aria-label="Settings"
          onClick={() => props.onOpenSettings()}
        >
          <Icon name="sliders" size={22} />
        </button>
      </header>
      <h1 class="display hero-title">What shall we knead today?</h1>
    </Show>

    <section class="dropzone" aria-label="Drop zone">
      <CatPeek class="dropzone-cat" whiskers={!props.compact} />
      <Show
        when={props.compact}
        fallback={
          <>
            <h2 class="display dropzone-title">Drop files on the kitty</h2>
            <p class="dropzone-hint">Video, audio or images — one file or a whole litter.</p>
          </>
        }
      >
        <h2 class="dropzone-title compact">Tap to pick files</h2>
        <p class="dropzone-hint">From Photos, Files or a share sheet</p>
      </Show>
      <button type="button" class="btn btn-primary" onClick={() => props.onOpenFile()}>
        Choose files
      </button>
      <Show when={!props.compact}>
        <ul class="chips" aria-label="Supported formats">
          <For each={SUPPORTED_CHIPS}>{(chip) => <li class="chip">{chip}</li>}</For>
        </ul>
      </Show>
    </section>

    <section>
      <h2 class="section-title">Quick recipes</h2>
      <div class="recipes">
        <For each={RECIPES}>
          {(recipe) => (
            <button
              type="button"
              class="recipe"
              data-tone={recipe.tone}
              onClick={() => props.onOpenFile(recipe.format)}
            >
              <span class="recipe-title">{recipe.title}</span>
              <span class="recipe-detail">
                {props.compact ? recipe.shortDetail : recipe.detail}
              </span>
              <span class="recipe-route">{recipe.route}</span>
            </button>
          )}
        </For>
      </div>
    </section>

    <Show
      when={props.compact}
      fallback={
        <section class="card recent">
          <h2 class="section-title">Recently purred</h2>
          <ul class="recent-list">
            <For each={RECENT}>
              {(item) => (
                <li class="recent-item">
                  <RecentIcon item={item} />
                  <div class="item-main">
                    <span class="item-name">{item.name}</span>
                    <span class="item-meta">
                      {item.route} · {item.from} → {item.to}
                    </span>
                  </div>
                  <span class="item-meta">{item.when}</span>
                </li>
              )}
            </For>
          </ul>
        </section>
      }
    >
      <button type="button" class="card recent-item recent-compact">
        <RecentIcon item={LATEST} />
        <span class="item-main">
          <span class="item-name">{LATEST.name}</span>
          <span class="item-meta">
            {LATEST.from} → {LATEST.to} · {LATEST.when}
          </span>
        </span>
        <Icon name="chevron-right" size={22} class="muted-icon" />
      </button>
    </Show>
  </div>
);
