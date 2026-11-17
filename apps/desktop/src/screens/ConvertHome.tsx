import { type Component, For, onMount, Show } from "solid-js";
import { CatPeek } from "../components/Cat";
import { Icon } from "../components/Icon";
import { Brand } from "../components/Navigation";
import { RecipeGrid } from "../components/RecipeGrid";
import { dragIcon, startDragOnMove } from "../lib/drag";
import { baseName, formatBytes, formatWhen } from "../lib/format";
import { KIND_TONE, label } from "../lib/formats";
import {
  checkHistory,
  chooseFiles,
  dragOut,
  type HistoryItem,
  navigate,
  reveal,
  state,
} from "../lib/store";

type Props = {
  compact: boolean;
  /** Files are being dragged over the window. */
  dropHover: boolean;
};

const KIND_ICON = { video: "video", audio: "music", image: "image", unknown: "paw" } as const;

export const RecentRow: Component<{ item: HistoryItem }> = (props) => {
  const missing = () => state.missing.includes(props.item.output);
  const drag = (event: PointerEvent) => {
    if (missing()) return;
    const { output, format, kind } = props.item;
    startDragOnMove(event, () => void dragOut(output, dragIcon(format, kind)));
  };
  return (
    <li
      class="recent-item"
      classList={{ missing: missing(), "drag-out": !missing() }}
      title={missing() ? undefined : "Drag into another app or folder"}
      onPointerDown={drag}
    >
      <span class="tile" data-tone={KIND_TONE[props.item.kind]}>
        <Icon name={KIND_ICON[props.item.kind]} size={18} />
      </span>
      <div class="item-main">
        <span class="item-name">
          {props.item.pictures
            ? `${props.item.pictures} pictures → ${props.item.name}`
            : props.item.name}
        </span>
        <span class="item-meta">
          {label(props.item.format)} · {formatBytes(props.item.inputBytes)} →{" "}
          {formatBytes(props.item.outputBytes)} ·{" "}
          {missing() ? "Moved or deleted" : formatWhen(props.item.finishedAt)}
        </span>
      </div>
      <button
        type="button"
        class="icon-btn plain"
        aria-label={`Show ${baseName(props.item.output)} in folder`}
        title={missing() ? "No longer where it was saved" : "Show in folder"}
        disabled={missing()}
        onClick={() => void reveal(props.item.output)}
      >
        <Icon name="folder" size={18} />
      </button>
    </li>
  );
};

export const ConvertHome: Component<Props> = (props) => {
  onMount(() => void checkHistory());
  return (
    <div class="page">
      <Show
        when={props.compact}
        fallback={
          <header class="page-header">
            <div>
              <h1 class="display page-title">Convert</h1>
              <p class="subtitle">Drop a file and we'll knead it into shape.</p>
            </div>
          </header>
        }
      >
        <header class="mobile-top">
          <Brand />
          <button
            type="button"
            class="icon-btn round"
            aria-label="Settings"
            onClick={() => navigate("settings")}
          >
            <Icon name="sliders" size={22} />
          </button>
        </header>
        <h1 class="display hero-title">What shall we knead today?</h1>
      </Show>

      <Show when={state.ffmpeg && !state.ffmpeg.found}>
        <button type="button" class="banner" onClick={() => navigate("settings")}>
          <Icon name="alert" size={20} />
          <span>
            <strong>ffmpeg is missing.</strong> The kitty needs it to convert anything.
          </span>
          <span class="banner-cta">
            How to fix <Icon name="arrow-right" size={16} />
          </span>
        </button>
      </Show>

      <section class="dropzone" classList={{ hover: props.dropHover }} aria-label="Drop zone">
        <CatPeek class="dropzone-cat" whiskers={!props.compact} />
        <h2 class="display dropzone-title">
          {props.dropHover ? "Let go, I've got it!" : "Drop files on the kitty"}
        </h2>
        <p class="dropzone-hint">Video, audio or images — one file or a whole litter.</p>
        <button type="button" class="btn btn-primary" onClick={() => void chooseFiles()}>
          Choose files
        </button>
      </section>

      <section>
        <h2 class="section-title">Quick recipes</h2>
        <RecipeGrid />
      </section>

      <section class="card recent">
        <div class="row-between">
          <h2 class="section-title">Recently purred</h2>
          <Show when={state.history.length > 3}>
            <button type="button" class="link-btn" onClick={() => navigate("history")}>
              See all
            </button>
          </Show>
        </div>
        <Show
          when={state.history.length > 0}
          fallback={
            <p class="item-meta empty-note">Nothing yet. Your first conversion lands here.</p>
          }
        >
          <ul class="recent-list">
            <For each={state.history.slice(0, 3)}>{(item) => <RecentRow item={item} />}</For>
          </ul>
        </Show>
      </section>
    </div>
  );
};
