import { createSignal, Match, Show, Switch } from "solid-js";
import { Sidebar, type Tab, TabBar } from "./components/Navigation";
import { createMediaQuery } from "./lib/media";
import { QUEUE_COUNT, type TargetFormat } from "./lib/mock";
import { ConvertFile } from "./screens/ConvertFile";
import { ConvertHome } from "./screens/ConvertHome";
import { Placeholder } from "./screens/Placeholder";
import { QueueScreen } from "./screens/QueueScreen";
import "./styles.css";

type Screen = Tab | "file";

/**
 * UI mockup of the CatConvert design. Screens render static sample data from
 * `lib/mock.ts`; nothing talks to the Tauri backend yet.
 */
export default function App() {
  const compact = createMediaQuery("(max-width: 720px)");
  const [screen, setScreen] = createSignal<Screen>("convert");
  const [format, setFormat] = createSignal<TargetFormat>("MP4");

  const activeTab = (): Tab => {
    const current = screen();
    return current === "file" ? "convert" : current;
  };
  const showTabBar = () => compact() && screen() !== "file";

  const openFile = (target: TargetFormat = "MP4") => {
    setFormat(target);
    setScreen("file");
  };

  return (
    <div class="shell" classList={{ compact: compact() }}>
      <Show when={!compact()}>
        <Sidebar active={activeTab()} queueCount={QUEUE_COUNT} onNavigate={setScreen} />
      </Show>
      <main class="content" classList={{ "with-tabbar": showTabBar() }}>
        <Switch>
          <Match when={screen() === "convert"}>
            <ConvertHome
              compact={compact()}
              onOpenFile={openFile}
              onOpenSettings={() => setScreen("settings")}
            />
          </Match>
          <Match when={screen() === "file"}>
            <ConvertFile
              compact={compact()}
              format={format()}
              onFormatChange={setFormat}
              onBack={() => setScreen("convert")}
              onStart={() => setScreen("queue")}
            />
          </Match>
          <Match when={screen() === "queue"}>
            <QueueScreen compact={compact()} />
          </Match>
          <Match when={screen() === "recipes"}>
            <Placeholder
              title="Recipes"
              message="Saved presets will live here. For now, try a quick recipe on Convert."
            />
          </Match>
          <Match when={screen() === "history"}>
            <Placeholder title="History" message="Everything you've converted will show up here." />
          </Match>
          <Match when={screen() === "settings"}>
            <Placeholder
              title="Settings"
              message="Output folder, ffmpeg path and defaults are coming soon."
            />
          </Match>
        </Switch>
      </main>
      <Show when={showTabBar()}>
        <TabBar active={activeTab()} onNavigate={setScreen} />
      </Show>
    </div>
  );
}
