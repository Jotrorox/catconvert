import { type Component, For, Show } from "solid-js";
import type { FfmpegStatus } from "../lib/api";
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
  ffmpeg: FfmpegStatus | null;
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
      <button
        type="button"
        class="status-card"
        data-state={props.ffmpeg === null ? "checking" : props.ffmpeg.found ? "ready" : "missing"}
        onClick={() => props.onNavigate("settings")}
      >
        <span class="status-dot" />
        <span class="status-text">
          <Show
            when={props.ffmpeg}
            fallback={
              <>
                <strong>Sniffing for ffmpeg…</strong>
                <span>One moment</span>
              </>
            }
          >
            {(status) => (
              <Show
                when={status().found}
                fallback={
                  <>
                    <strong>ffmpeg missing</strong>
                    <span>Tap to fix</span>
                  </>
                }
              >
                <strong>ffmpeg {status().version}</strong>
                <span>Ready to convert</span>
              </Show>
            )}
          </Show>
        </span>
      </button>
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

export const TabBar: Component<Omit<Props, "ffmpeg">> = (props) => (
  <nav class="tabbar" aria-label="Main">
    <For each={TABS}>
      {(tab) => (
        <button
          type="button"
          class="tab"
          aria-current={props.active === tab.id ? "page" : undefined}
          onClick={() => props.onNavigate(tab.id)}
        >
          <span class="tab-icon">
            <Icon name={tab.icon} size={26} />
            <Show when={tab.id === "queue" && props.queueCount > 0}>
              <span class="tab-badge">{props.queueCount}</span>
            </Show>
          </span>
          {tab.label}
        </button>
      )}
    </For>
  </nav>
);
