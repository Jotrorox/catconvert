import { type Component, For, Show } from "solid-js";
import { dismissToast, state } from "../lib/store";
import { Icon } from "./Icon";

/** Little notes that slide up from the bottom and leave on their own. */
export const Toasts: Component = () => (
  <div class="toasts" role="status" aria-live="polite">
    <For each={state.toasts}>
      {(toast) => (
        <div class="toast" data-tone={toast.tone === "error" ? "error" : undefined}>
          <Icon name={toast.tone === "error" ? "alert" : "paw"} size={18} class="toast-icon" />
          <span class="toast-text">{toast.message}</span>
          <Show when={toast.action}>
            {(action) => (
              <button
                type="button"
                class="toast-action"
                onClick={() => {
                  action().run();
                  dismissToast(toast.id);
                }}
              >
                {action().label}
              </button>
            )}
          </Show>
          <button
            type="button"
            class="icon-btn plain toast-close"
            aria-label="Dismiss"
            onClick={() => dismissToast(toast.id)}
          >
            <Icon name="close" size={14} />
          </button>
        </div>
      )}
    </For>
  </div>
);
