import { type Component, For, Show } from "solid-js";
import { CatLogo } from "./Cat";
import { Icon, type IconName } from "./Icon";

export type Tab = "convert" | "queue" | "recipes" | "history" | "settings";

const TABS: readonly { id: Tab; label: string; icon: IconName }[] = [
  { id: "convert", label: "Convert", icon: "convert" },
  { id: "queue", label: "Queue", icon: "queue" },
  { id: "recipes", label: "Recipes", icon: "star" },
  { id: "history", label: "History", icon: "clock" },
];

type Props = {
  active: Tab;
  queueCount: number;
  onNavigate: (tab: Tab) => void;
};

export const Brand: Component = () => (
  <span class="brand">
    <CatLogo class="brand-logo" />
    CatConvert
  </span>
);

export const Sidebar: Component<Props> = (props) => (
  <aside class="sidebar">
    <Brand />
    <nav class="nav" aria-label="Main">
      <For each={TABS}>
        {(tab) => (
          <button
            type="button"
            class="nav-item"
            aria-current={props.active === tab.id ? "page" : undefined}
            onClick={() => props.onNavigate(tab.id)}
          >
            <Icon name={tab.icon} />
            {tab.label}
            <Show when={tab.id === "queue" && props.queueCount > 0}>
              <span class="nav-badge" role="status" aria-label={`${props.queueCount} in queue`}>
                {props.queueCount}
              </span>
            </Show>
          </button>
        )}
      </For>
    </nav>
    <div class="sidebar-footer">
      <div class="status-card">
        <span class="status-dot" />
        <div>
          <strong>ffmpeg found</strong>
          <span>Ready to convert</span>
        </div>
      </div>
      <button
        type="button"
        class="nav-item"
        aria-current={props.active === "settings" ? "page" : undefined}
        onClick={() => props.onNavigate("settings")}
      >
        <Icon name="sliders" />
        Settings
      </button>
    </div>
  </aside>
);

export const TabBar: Component<Omit<Props, "queueCount">> = (props) => (
  <nav class="tabbar" aria-label="Main">
    <For each={TABS}>
      {(tab) => (
        <button
          type="button"
          class="tab"
          aria-current={props.active === tab.id ? "page" : undefined}
          onClick={() => props.onNavigate(tab.id)}
        >
          <Icon name={tab.icon} size={26} />
          {tab.label}
        </button>
      )}
    </For>
  </nav>
);
