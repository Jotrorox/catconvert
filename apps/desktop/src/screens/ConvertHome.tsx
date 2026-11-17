import { type Component, createSignal, For, onMount, Show } from "solid-js";
import { CatPeek } from "../components/Cat";
import { ContextMenu, type MenuItem } from "../components/ContextMenu";
import { Icon } from "../components/Icon";
import { Brand } from "../components/Navigation";
import { PlayButton } from "../components/PlayButton";
import { RecipeGrid } from "../components/RecipeGrid";
import { dragIcon, startDragOnMove } from "../lib/drag";
import { baseName, formatBytes, formatWhen } from "../lib/format";
import { formatKind, KIND_TONE, label } from "../lib/formats";
import {
  allRecipes,
  checkHistory,
  chooseFiles,
  convertAgain,
  dragOut,
  type HistoryItem,
  navigate,
  reconvertSource,
  reveal,
  state,
} from "../lib/store";

type Props = {
  compact: boolean;
  /** Files are being dragged over the window. */
  dropHover: boolean;
};

const KIND_ICON = { video: "video", audio: "music", image: "image", unknown: "paw" } as const;

/** What "Convert again" says about where it starts from, if anything. */
function reconvertHint(item: HistoryItem): string | undefined {
  switch (reconvertSource(item)) {
    case "output":
      return item.sequence
        ? "The pictures are gone; opens the animation instead"
        : "The original is gone; opens the converted file instead";
    case "none":
      return item.sequence
        ? "The pictures and the animation are both gone"
        : "The original and the converted file are both gone";
    default:
      return undefined;
  }
}

/**
 * A finished conversion. With `actions` (History) it can also be converted
 * again, from a button or by right-clicking the row.
 */
export const RecentRow: Component<{ item: HistoryItem; actions?: boolean }> = (props) => {
  const missing = () => state.missing.includes(props.item.output);
  const source = () => (props.actions ? reconvertSource(props.item) : null);
  const [menu, setMenu] = createSignal<{ x: number; y: number } | null>(null);
  const drag = (event: PointerEvent) => {
    if (missing()) return;
    const { output, format, kind } = props.item;
    startDragOnMove(event, () => void dragOut(output, dragIcon(format, kind)));
  };
  /** The kind of file a recipe would be applied to, if that file is still there. */
  const recipeKind = () => {
    const item = props.item;
    if (source() === "original" && !item.sequence) return item.kind;
    if (source() !== null && !missing()) return formatKind(item.format, item.kind);
    return null;
  };
  const menuItems = (): MenuItem[] => {
    const item = props.item;
    const items: MenuItem[] = [];
    if (source() !== null) {
      items.push({
        label: "Convert again…",
        run: () => void convertAgain(item.id),
        disabled: source() === "none",
        hint: reconvertHint(item),
      });
      const kind = recipeKind();
      const recipes = kind ? allRecipes().filter((r) => r.accepts.includes(kind)) : [];
      if (recipes.length > 0) {
        items.push({ heading: "Convert again with" });
        for (const recipe of recipes) {
          items.push({ label: recipe.title, run: () => void convertAgain(item.id, recipe.id) });
        }
      }
      items.push({ separator: true });
    }
    items.push({
      label: "Show in folder",
      run: () => void reveal(item.output),
      disabled: missing(),
      hint: missing() ? "No longer where it was saved" : undefined,
    });
    return items;
  };
  const openMenu = (event: MouseEvent) => {
    if (!props.actions) return;
    event.preventDefault();
    setMenu({ x: event.clientX, y: event.clientY });
  };
  return (
    <>
      <li
        class="recent-item"
        classList={{ missing: missing(), "drag-out": !missing() }}
        title={missing() ? undefined : "Drag into another app or folder"}
        onPointerDown={drag}
        onContextMenu={openMenu}
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
            {label(props.item.format)}
            {props.item.recipe ? ` · ${props.item.recipe}` : ""} ·{" "}
            {formatBytes(props.item.inputBytes)} → {formatBytes(props.item.outputBytes)} ·{" "}
            {missing() ? "Moved or deleted" : formatWhen(props.item.finishedAt)}
            <Show when={props.item.replaced}>
              {" "}
              <span
                class="replace-badge inline"
                title={
                  props.item.keptAs
                    ? `The original was kept as ${baseName(props.item.keptAs)}`
                    : "The original went to the Trash"
                }
              >
                Replaced original
              </span>
            </Show>
          </span>
        </div>
        <Show when={formatKind(props.item.format, props.item.kind) === "audio"}>
          <PlayButton
            path={props.item.output}
            name={baseName(props.item.output)}
            format={props.item.format}
            disabled={missing()}
          />
        </Show>
        <Show when={source() !== null}>
          <button
            type="button"
            class="icon-btn plain"
            aria-label={`Convert ${props.item.name} again`}
            title={reconvertHint(props.item) ?? "Convert again with different settings"}
            disabled={source() === "none"}
            onClick={() => void convertAgain(props.item.id)}
          >
            <Icon name="convert" size={18} />
          </button>
        </Show>
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
      {/* Outside the row, so presses in the menu don't start a drag or a new menu. */}
      <Show when={menu()}>
        {(at) => (
          <ContextMenu
            x={at().x}
            y={at().y}
            label={props.item.name}
            items={menuItems()}
            onClose={() => setMenu(null)}
          />
        )}
      </Show>
    </>
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
        <p class="dropzone-hint">Video, audio or images. One file or a whole litter.</p>
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
