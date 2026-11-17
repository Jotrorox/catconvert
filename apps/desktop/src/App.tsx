import { createSignal, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { CatPeek } from "./components/Cat";
import { Sidebar, type Tab, TabBar } from "./components/Navigation";
import { Toasts } from "./components/Toasts";
import { inTauri, onFileDrop, onJobProgress, onOpenedFiles, type UnlistenFn } from "./lib/api";
import { createMediaQuery } from "./lib/media";
import { overlayTitleBar } from "./lib/platform";
import {
  activeCount,
  applyProgress,
  checkForUpdates,
  checkHistory,
  loadAppVersion,
  navigate,
  openPaths,
  refreshFfmpeg,
  state,
} from "./lib/store";
import { ConvertHome } from "./screens/ConvertHome";
import { Editor } from "./screens/Editor";
import { HistoryScreen } from "./screens/HistoryScreen";
import { QueueScreen } from "./screens/QueueScreen";
import { RecipesScreen } from "./screens/RecipesScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import "./styles.css";

export default function App() {
  const compact = createMediaQuery("(max-width: 720px)");
  const [dropHover, setDropHover] = createSignal(false);

  const activeTab = (): Tab => (state.screen === "edit" ? "convert" : state.screen);
  const showTabBar = () => compact() && state.screen !== "edit";

  onMount(() => {
    void refreshFfmpeg();
    if (!inTauri) return;
    void loadAppVersion();
    void checkForUpdates({ quiet: true });
    // Files may have been moved or deleted while the window was in the background.
    const recheck = () => void checkHistory();
    window.addEventListener("focus", recheck);
    onCleanup(() => window.removeEventListener("focus", recheck));
    const unlisten: Promise<UnlistenFn>[] = [
      onJobProgress(applyProgress),
      onFileDrop({ onHover: setDropHover, onDrop: openPaths }),
      // "Open with", the Finder Quick Action, "Send to"…
      onOpenedFiles(openPaths),
    ];
    onCleanup(() => {
      for (const p of unlisten) void p.then((stop) => stop());
    });
  });

  return (
    <div class="shell" classList={{ compact: compact() }}>
      <Show when={overlayTitleBar}>
        <div class="titlebar" data-tauri-drag-region />
      </Show>
      <Show when={!compact()}>
        <Sidebar active={activeTab()} queueCount={activeCount()} onNavigate={navigate} />
      </Show>
      <main class="content" classList={{ "with-tabbar": showTabBar() }}>
        <Switch>
          <Match when={state.screen === "convert"}>
            <ConvertHome compact={compact()} dropHover={dropHover()} />
          </Match>
          <Match when={state.screen === "edit"}>
            <Editor compact={compact()} />
          </Match>
          <Match when={state.screen === "queue"}>
            <QueueScreen compact={compact()} />
          </Match>
          <Match when={state.screen === "recipes"}>
            <RecipesScreen />
          </Match>
          <Match when={state.screen === "history"}>
            <HistoryScreen />
          </Match>
          <Match when={state.screen === "settings"}>
            <SettingsScreen />
          </Match>
        </Switch>
      </main>
      <Show when={showTabBar()}>
        <TabBar active={activeTab()} queueCount={activeCount()} onNavigate={navigate} />
      </Show>
      <Show when={dropHover() && state.screen !== "convert"}>
        <div class="drop-overlay" aria-hidden="true">
          <div class="drop-overlay-card">
            <CatPeek class="drop-overlay-cat" />
            <p class="display">Drop to add</p>
          </div>
        </div>
      </Show>
      <Toasts />
    </div>
  );
}
