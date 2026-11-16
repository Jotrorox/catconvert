import { type Component, createSignal, For, Match, Show, Switch } from "solid-js";
import { CatPeek } from "../components/Cat";
import { Icon } from "../components/Icon";
import { formatBytes } from "../lib/format";
import { FORMAT_GROUPS, label } from "../lib/formats";
import {
  activeCount,
  checkForUpdates,
  installUpdate,
  isAvailable,
  refreshFfmpeg,
  restartApp,
  state,
  toast,
  type UpdateState,
} from "../lib/store";

const INSTALL = [
  { os: "macOS", command: "brew install ffmpeg" },
  { os: "Windows", command: "winget install ffmpeg" },
  { os: "Linux", command: "sudo apt install ffmpeg" },
];

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

  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1 class="display page-title">Settings</h1>
          <p class="subtitle">CatConvert uses the ffmpeg on your computer. Nothing leaves it.</p>
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
            <p class="item-meta mono path">{state.ffmpeg?.path}</p>
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

      <section class="card">
        <h2 class="section-title">Where files go</h2>
        <p class="item-meta">
          Converted files are saved next to the original, as <code>name.mp4</code>. If that name is
          taken we add a number, so nothing is ever overwritten.
        </p>
      </section>

      <UpdatesCard />
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
