import {
  type Component,
  createSignal,
  For,
  Match,
  onCleanup,
  onMount,
  Show,
  Switch,
} from "solid-js";
import { Portal } from "solid-js/web";

export type MenuItem =
  | {
      label: string;
      run: () => void;
      disabled?: boolean;
      /** A second, quieter line: what the item will do, or why it can't. */
      hint?: string | undefined;
    }
  | { heading: string }
  | { separator: true };

/** Room kept between the menu and the window's edges. */
const MARGIN = 8;

/**
 * A menu at the pointer, opened by right-clicking. Arrow keys move between
 * items; Escape, Tab, a click elsewhere, scrolling or leaving the window
 * closes it.
 */
export const ContextMenu: Component<{
  x: number;
  y: number;
  label: string;
  items: MenuItem[];
  onClose: () => void;
}> = (props) => {
  let menu!: HTMLDivElement;
  const [position, setPosition] = createSignal({ left: props.x, top: props.y });
  const enabled = () =>
    Array.from(menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)'));

  onMount(() => {
    const rect = menu.getBoundingClientRect();
    setPosition({
      left: Math.max(MARGIN, Math.min(props.x, window.innerWidth - rect.width - MARGIN)),
      top: Math.max(MARGIN, Math.min(props.y, window.innerHeight - rect.height - MARGIN)),
    });
    enabled()[0]?.focus({ preventScroll: true });

    const away = (event: PointerEvent) => {
      if (!(event.target instanceof Node) || !menu.contains(event.target)) props.onClose();
    };
    const close = () => props.onClose();
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    onCleanup(() => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
    });
  });

  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      props.onClose();
      return;
    }
    const items = enabled();
    if (items.length === 0) return;
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "ArrowDown"
        ? (at + 1) % items.length
        : event.key === "ArrowUp"
          ? (at - 1 + items.length) % items.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? items.length - 1
              : null;
    if (next === null) return;
    event.preventDefault();
    items[next]?.focus();
  };

  return (
    <Portal>
      <div
        ref={menu}
        class="context-menu"
        role="menu"
        aria-label={props.label}
        style={{ left: `${position().left}px`, top: `${position().top}px` }}
        onKeyDown={onKey}
        onContextMenu={(event) => event.preventDefault()}
      >
        <For each={props.items}>
          {(item) => (
            <Switch>
              <Match when={"separator" in item}>
                <hr class="menu-separator" />
              </Match>
              <Match when={"heading" in item && item}>
                {(heading) => (
                  <div class="menu-heading" role="presentation">
                    {(heading() as { heading: string }).heading}
                  </div>
                )}
              </Match>
              <Match when={"label" in item && item}>
                {(entry) => {
                  const it = entry() as Extract<MenuItem, { label: string }>;
                  return (
                    <button
                      type="button"
                      role="menuitem"
                      class="menu-item"
                      disabled={it.disabled}
                      onClick={() => {
                        props.onClose();
                        it.run();
                      }}
                    >
                      <span>{it.label}</span>
                      <Show when={it.hint}>
                        <span class="menu-hint">{it.hint}</span>
                      </Show>
                    </button>
                  );
                }}
              </Match>
            </Switch>
          )}
        </For>
      </div>
    </Portal>
  );
};
