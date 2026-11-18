import { type Component, createSignal, For, Match, onMount, Show, Switch } from "solid-js";
import { CatPeek } from "../components/Cat";
import { Icon } from "../components/Icon";
import { ReplaceConfirm } from "../components/ReplaceConfirm";
import { type OutputFormat, thirdPartyLicenses } from "../lib/api";
import { formatBytes } from "../lib/format";
import {
  FORMAT_GROUPS,
  type FormatGroup,
  label,
  METADATA_CHOICES,
  type OutputChoice,
  QUALITY_STEPS,
  qualityStep,
} from "../lib/formats";
import { os } from "../lib/platform";
import {
  activeCount,
  checkForUpdates,
  chooseOutputFolder,
  confirmReplace,
  installUpdate,
  isAvailable,
  loadQuickAction,
  refreshFfmpeg,
  restartApp,
  setOutput,
  setOutputByKind,
  setTheme,
  state,
  toast,
  toggleQuickAction,
  type UpdateState,
  updateSettings,
} from "../lib/store";
import { THEME_CHOICES } from "../lib/theme";

const INSTALL = [
  { os: "macOS", command: "brew install ffmpeg" },
  { os: "Windows", command: "winget install ffmpeg" },
  { os: "Linux", command: "sudo apt install ffmpeg" },
];

const BECOMES: Record<FormatGroup["kind"], string> = {
  video: "Videos become",
  audio: "Sound becomes",
  image: "Pictures become",
};

/** What freshly opened files start with. */
const StartingSettings: Component = () => {
  const settings = () => state.settings;
  const setFormat = (kind: FormatGroup["kind"], value: string) =>
    updateSettings({
      formats: { ...settings().formats, [kind]: value === "" ? null : (value as OutputFormat) },
    });

  return (
    <section class="card defaults">
      <h2 class="section-title">Starting settings</h2>
      <p class="item-meta">
        What a freshly opened file starts with. Recipes and the editor can still change anything.
      </p>
      <div class="three-col">
        <For each={FORMAT_GROUPS}>
          {(group) => (
            <label class="field">
              <span class="field-label">{BECOMES[group.kind]}</span>
              <select class="select" onChange={(e) => setFormat(group.kind, e.currentTarget.value)}>
                <option value="" selected={settings().formats[group.kind] === null}>
                  Let the kitty pick
                </option>
                <For each={group.formats}>
                  {(format) => (
                    <option
                      value={format}
                      selected={settings().formats[group.kind] === format}
                      disabled={state.ffmpeg?.found === true && !isAvailable(format)}
                    >
                      {label(format)}
                    </option>
                  )}
                </For>
              </select>
            </label>
          )}
        </For>
      </div>
      <div class="field">
        <label class="field-label" for="default-quality">
          Quality
        </label>
        <input
          id="default-quality"
          class="range"
          type="range"
          min="0"
          max="100"
          value={settings().quality}
          style={{ "--pct": `${settings().quality}%` }}
          onInput={(e) => updateSettings({ quality: e.currentTarget.valueAsNumber })}
        />
        <div class="range-scale">
          <For each={QUALITY_STEPS}>
            {(name) => (
              <span classList={{ active: qualityStep(settings().quality) === name }}>{name}</span>
            )}
          </For>
        </div>
      </div>
      <fieldset class="field metadata-field">
        <legend class="field-label">Metadata</legend>
        <div class="format-chips">
          <For each={METADATA_CHOICES}>
            {(choice) => (
              <button
                type="button"
                class="format-chip"
                aria-pressed={settings().metadata === choice.mode}
                onClick={() => updateSettings({ metadata: choice.mode })}
              >
                {choice.label}
              </button>
            )}
          </For>
        </div>
        <span class="item-meta">
          Dates, place and camera details, where the format allows. JPG, WebP and AVIF never carry
          them.
        </span>
      </fieldset>
    </section>
  );
};

/** Light, dark, or the same as the system. */
const Appearance: Component = () => (
  <section class="card defaults">
    <h2 class="section-title">Appearance</h2>
    <fieldset class="field theme-field">
      <legend class="field-label">Theme</legend>
      <div class="format-chips">
        <For each={THEME_CHOICES}>
          {(item) => (
            <button
              type="button"
              class="format-chip"
              aria-pressed={state.theme === item.choice}
              onClick={() => setTheme(item.choice)}
            >
              {item.label}
            </button>
          )}
        </For>
      </div>
      <span class="item-meta">
        Same as system turns dark and light along with your computer, even while CatConvert is open.
      </span>
    </fieldset>
  </section>
);

const OUTPUT_CHOICES: readonly { choice: OutputChoice; label: string; detail: string }[] = [
  {
    choice: "nextToInput",
    label: "Next to the original",
    detail:
      "Saved next to the original, as name.mp4. If that name is taken we add a number, so nothing is overwritten.",
  },
  {
    choice: "folder",
    label: "In a folder",
    detail: "Everything lands in one folder, under a free name.",
  },
  {
    choice: "replaceInput",
    label: "Replace the original",
    detail:
      "The converted file takes the original's place and the original goes to the Trash. A new format gets its own extension, like clip.mov becoming clip.mp4. Animations made from several pictures are saved next to them.",
  },
];

/** Where converted files go by default. */
const OutputSettings: Component = () => {
  const settings = () => state.settings;
  const [confirming, setConfirming] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const choose = async (choice: OutputChoice) => {
    setConfirming(false);
    if (choice === "replaceInput" && !settings().replaceConfirmed) {
      setConfirming(true);
      return;
    }
    setBusy(true);
    await setOutput(choice);
    setBusy(false);
  };
  const detail = () => OUTPUT_CHOICES.find((c) => c.choice === settings().output)?.detail;

  return (
    <section class="card defaults">
      <h2 class="section-title">Where files go</h2>
      <fieldset class="field">
        <legend class="field-label">Save converted files</legend>
        <div class="format-chips">
          <For each={OUTPUT_CHOICES}>
            {(item) => (
              <button
                type="button"
                class="format-chip"
                aria-pressed={settings().output === item.choice}
                disabled={busy()}
                onClick={() => void choose(item.choice)}
              >
                {item.label}
              </button>
            )}
          </For>
        </div>
      </fieldset>
      <Show when={confirming()}>
        <ReplaceConfirm
          onConfirm={() => {
            confirmReplace();
            void choose("replaceInput");
          }}
          onCancel={() => setConfirming(false)}
        />
      </Show>
      <p class="item-meta">{detail()}</p>
      <Show when={settings().output === "folder" && settings().outputFolder}>
        {(folder) => (
          <>
            <div class="folder-row">
              <Icon name="folder" size={18} />
              <span class="mono path folder-path" title={folder()}>
                {folder()}
              </span>
              <button
                type="button"
                class="btn btn-outline"
                onClick={() => void chooseOutputFolder()}
              >
                Change…
              </button>
            </div>
            <div class="toggle-row">
              <div>
                <span class="toggle-title" id="output-by-kind">
                  Sort by kind
                </span>
                <span class="item-meta">Into Videos, Pictures and Audio folders inside it</span>
              </div>
              <button
                type="button"
                class="switch"
                role="switch"
                aria-checked={settings().outputByKind}
                aria-labelledby="output-by-kind"
                onClick={() => setOutputByKind(!settings().outputByKind)}
              />
            </div>
          </>
        )}
      </Show>
      <p class="item-meta">The editor can change this for one conversion.</p>
    </section>
  );
};

const OPEN_WITH: Record<typeof os, string> = {
  macos:
    "Right-click a photo, video or song in Finder and choose Open With › CatConvert, or drop it on CatConvert in the Dock.",
  windows:
    "Right-click a photo, video or song in Explorer and choose Open with › CatConvert, or Send to › CatConvert for several at once.",
  linux:
    "Right-click a photo, video or song in your file manager and choose Open With › CatConvert.",
  other: "Open photos, videos and songs with CatConvert from your file manager.",
};

/** Starting conversions from Finder or Explorer, and dragging results out. */
const OtherApps: Component = () => {
  const [busy, setBusy] = createSignal(false);
  const toggle = async () => {
    setBusy(true);
    await toggleQuickAction();
    setBusy(false);
  };
  onMount(() => void loadQuickAction());

  return (
    <section class="card defaults">
      <h2 class="section-title">From other apps</h2>
      <p class="item-meta">{OPEN_WITH[os]}</p>
      <Show when={state.quickAction?.supported}>
        <div class="toggle-row">
          <div>
            <span class="toggle-title" id="quick-action">
              Finder Quick Action
            </span>
            <span class="item-meta">
              Adds Convert with CatConvert to Quick Actions when you right-click files
            </span>
          </div>
          <button
            type="button"
            class="switch"
            role="switch"
            aria-checked={state.quickAction?.installed ?? false}
            aria-labelledby="quick-action"
            disabled={busy()}
            onClick={() => void toggle()}
          />
        </div>
      </Show>
      <p class="item-meta">
        Finished files can be dragged from the Queue or History straight into another app or folder.
      </p>
    </section>
  );
};

export const SettingsScreen: Component = () => {
  const [looking, setLooking] = createSignal(false);
  const lookAgain = async () => {
    setLooking(true);
    const status = await refreshFfmpeg(true);
    setLooking(false);
    toast(status.found ? `Found ffmpeg ${status.version}` : "Still no ffmpeg in sight", {
      tone: status.found ? "info" : "error",
    });
  };

  // undefined: not loaded yet, null: this build ships no third-party tools.
  const [licenses, setLicenses] = createSignal<string | null | undefined>();
  const [showLicenses, setShowLicenses] = createSignal(false);
  const toggleLicenses = async () => {
    if (licenses() === undefined) {
      setLicenses(await thirdPartyLicenses().catch(() => null));
    }
    setShowLicenses(!showLicenses());
  };

  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1 class="display page-title">Settings</h1>
          <p class="subtitle">Everything is converted on your computer. Nothing leaves it.</p>
        </div>
      </header>

      <section class="card ffmpeg-card" data-found={state.ffmpeg?.found ? "" : undefined}>
        <CatPeek sleepy={!state.ffmpeg?.found} class="ffmpeg-cat" />
        <div class="ffmpeg-body">
          <Show
            when={state.ffmpeg?.found}
            fallback={
              <>
                <h2 class="label-strong">ffmpeg is missing</h2>
                <p class="item-meta">
                  {state.ffmpeg?.error ?? "Checking…"} Install it, then look again:
                </p>
                <ul class="install-list">
                  <For each={INSTALL}>
                    {(item) => (
                      <li>
                        <span>{item.os}</span>
                        <code class="mono">{item.command}</code>
                      </li>
                    )}
                  </For>
                </ul>
              </>
            }
          >
            <h2 class="label-strong">ffmpeg {state.ffmpeg?.version}</h2>
            <Show
              when={state.ffmpeg?.bundled}
              fallback={<p class="item-meta mono path">{state.ffmpeg?.path}</p>}
            >
              <p class="item-meta">Built into CatConvert</p>
            </Show>
            <p class="item-meta">
              <Show
                when={state.ffmpeg?.libjxl}
                fallback={
                  <>
                    JPEG XL tools not found. For the full JPEG XL kit (animations, lossless JPEG
                    repacking) install libjxl: <code class="mono">brew install jpeg-xl</code>
                  </>
                }
              >
                {(version) => <>JPEG XL: libjxl {version()} (cjxl, djxl)</>}
              </Show>
            </p>
          </Show>
          <button
            type="button"
            class="btn btn-outline"
            disabled={looking()}
            onClick={() => void lookAgain()}
          >
            <Icon name="refresh" size={16} />
            {looking() ? "Looking…" : "Look again"}
          </button>
        </div>
      </section>

      <StartingSettings />

      <Show when={state.ffmpeg?.found}>
        <section class="card">
          <h2 class="section-title">What your ffmpeg can make</h2>
          <div class="format-groups">
            <For each={FORMAT_GROUPS}>
              {(group) => (
                <div class="format-group" data-tone={group.tone}>
                  <span class="format-group-label">{group.label}</span>
                  <div class="format-chips">
                    <For each={group.formats}>
                      {(format) => (
                        <span
                          class="format-chip static"
                          data-off={!isAvailable(format) || undefined}
                        >
                          {label(format)}
                        </span>
                      )}
                    </For>
                  </div>
                </div>
              )}
            </For>
          </div>
          <p class="item-meta settings-note">
            Greyed-out formats need an ffmpeg built with more encoders. JXL works with libjxl's
            tools or an ffmpeg built with libjxl.
          </p>
        </section>
      </Show>

      <OutputSettings />

      <OtherApps />

      <Appearance />

      <UpdatesCard />

      <section class="card">
        <h2 class="section-title">Open-source licenses</h2>
        <p class="item-meta">
          CatConvert comes with ffmpeg (GPL-2.0-or-later, built with x264 and x265) and libjxl's
          tools, which it runs as separate programs. Every release on GitHub includes their complete
          source code.
        </p>
        <p class="item-meta">
          H.264 and H.265 (HEVC) are covered by patents licensed through patent pools. CatConvert
          includes the open-source x264 and x265 encoders, but no patent license; whether you need
          one depends on where and how you use the videos.
        </p>
        <button
          type="button"
          class="btn btn-outline settings-note"
          aria-expanded={showLicenses()}
          onClick={() => void toggleLicenses()}
        >
          {showLicenses() ? "Hide licenses" : "Show licenses"}
        </button>
        <Show when={showLicenses()}>
          <Show
            when={licenses()}
            fallback={
              <p class="item-meta settings-note">
                This build doesn't include ffmpeg; it uses the one installed on your computer.
              </p>
            }
          >
            {(text) => <pre class="licenses mono">{text()}</pre>}
          </Show>
        </Show>
      </section>
    </div>
  );
};

type Status<S extends UpdateState["status"]> = Extract<UpdateState, { status: S }>;

function downloadText(progress: Status<"downloading">["progress"]): string {
  const done = formatBytes(progress.downloaded);
  return progress.total ? `${done} of ${formatBytes(progress.total)}` : done;
}

function downloadPct(progress: Status<"downloading">["progress"]): number | null {
  if (!progress.total) return null;
  return Math.min(100, Math.round((progress.downloaded / progress.total) * 100));
}

const UpdatesCard: Component = () => {
  const busy = () => activeCount() > 0;
  const update = () => state.update;
  const is = <S extends UpdateState["status"]>(status: S) => {
    const current = update();
    return current.status === status ? (current as Status<S>) : false;
  };

  const notes = (info: { notes: string | null }) => (
    <Show when={info.notes}>{(text) => <p class="update-notes">{text()}</p>}</Show>
  );
  const WaitNote = () => (
    <Show when={busy()}>
      <p class="item-meta">Waits until your conversions are done.</p>
    </Show>
  );

  return (
    <section class="card update-card" aria-labelledby="updates-title">
      <h2 class="section-title" id="updates-title">
        Updates
      </h2>
      <Show when={state.appVersion}>
        {(version) => <p class="item-meta">You have CatConvert {version()}.</p>}
      </Show>
      <div class="update-body">
        <Switch>
          <Match when={is("available")}>
            {(s) => (
              <>
                <p class="label-strong">CatConvert {s().info.version} is out</p>
                {notes(s().info)}
                <button
                  type="button"
                  class="btn btn-primary"
                  disabled={busy()}
                  onClick={() => void installUpdate()}
                >
                  <Icon name="sparkle" size={16} />
                  Download and install
                </button>
                <WaitNote />
              </>
            )}
          </Match>
          <Match when={is("downloading")}>
            {(s) => (
              <>
                <p class="label-strong">Fetching CatConvert {s().info.version}…</p>
                <div
                  class="progress update-progress"
                  role="progressbar"
                  aria-label="Downloading the update"
                  aria-valuemin="0"
                  aria-valuemax="100"
                  aria-valuenow={downloadPct(s().progress) ?? undefined}
                >
                  <div
                    class="progress-fill"
                    style={{ width: `${Math.max(downloadPct(s().progress) ?? 100, 4)}%` }}
                  />
                </div>
                <p class="item-meta">{downloadText(s().progress)}</p>
              </>
            )}
          </Match>
          <Match when={is("ready")}>
            {(s) => (
              <>
                <p class="label-strong">CatConvert {s().info.version} is installed</p>
                <p class="item-meta">Restart to start using it.</p>
                <button
                  type="button"
                  class="btn btn-primary"
                  disabled={busy()}
                  onClick={() => void restartApp()}
                >
                  <Icon name="refresh" size={16} />
                  Restart now
                </button>
                <WaitNote />
              </>
            )}
          </Match>
          <Match when={is("error")}>
            {(s) => (
              <>
                <p class="label-strong">
                  <Icon name="alert" size={16} class="update-error-icon" />
                  {s().info ? "The update didn't install" : "Couldn't check for updates"}
                </p>
                <p class="item-meta">{s().message}</p>
                <button
                  type="button"
                  class="btn btn-outline"
                  disabled={s().info !== null && busy()}
                  onClick={() => void (s().info ? installUpdate() : checkForUpdates())}
                >
                  <Icon name="refresh" size={16} />
                  Try again
                </button>
              </>
            )}
          </Match>
          <Match when={true}>
            <p class="item-meta">
              {is("current")
                ? "You're up to date."
                : "CatConvert looks for a new version each time it opens."}
            </p>
            <button
              type="button"
              class="btn btn-outline"
              disabled={update().status === "checking"}
              onClick={() => void checkForUpdates()}
            >
              <Icon name="refresh" size={16} />
              {update().status === "checking" ? "Checking…" : "Check for updates"}
            </button>
          </Match>
        </Switch>
      </div>
    </section>
  );
};
