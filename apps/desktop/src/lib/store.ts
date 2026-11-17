/**
 * App state: which screen is showing, the file being edited, files waiting
 * their turn, the conversion queue, the history of finished conversions, and
 * the user's starting settings and saved recipes.
 *
 * All backend calls go through `./api`; components only call the actions
 * exported here.
 */
import { batch } from "solid-js";
import { createStore, produce } from "solid-js/store";
import type {
  ConvertOptions,
  FfmpegStatus,
  JobId,
  JobProgress,
  JobStage,
  MediaInfo,
  OutputFormat,
  UpdateInfo,
} from "./api";
import * as api from "./api";
import { baseName } from "./format";
import {
  DEFAULTS,
  type Defaults,
  fitOptions,
  initialOptions,
  RECIPES,
  type Recipe,
  type RecipeId,
  recipeFrom,
} from "./formats";

export type Screen = "convert" | "edit" | "queue" | "recipes" | "history" | "settings";

export type Draft = {
  info: MediaInfo;
  options: ConvertOptions;
};

export type JobEntry = {
  id: JobId;
  name: string;
  input: MediaInfo;
  output: string;
  options: ConvertOptions;
  command: string;
  stage: JobStage;
  progress: number;
  speed: number | null;
  outputBytes: number | null;
  error: string | null;
  submittedAt: number;
  finishedAt: number | null;
};

export type HistoryItem = {
  id: string;
  name: string;
  output: string;
  format: OutputFormat;
  kind: MediaInfo["kind"];
  inputBytes: number;
  outputBytes: number;
  finishedAt: number;
};

export type Toast = {
  id: number;
  message: string;
  tone: "info" | "error";
  action?: { label: string; run: () => void };
};

/** Where the in-app updater is. */
export type UpdateState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "current" }
  | { status: "available"; info: UpdateInfo }
  | { status: "downloading"; info: UpdateInfo; progress: api.DownloadProgress }
  /** Installed; the new version starts after a restart. */
  | { status: "ready"; info: UpdateInfo }
  /** `info` is set when the check worked but downloading or installing failed. */
  | { status: "error"; message: string; info: UpdateInfo | null };

type State = {
  screen: Screen;
  /** The running app's version (null until known, or outside the Tauri shell). */
  appVersion: string | null;
  update: UpdateState;
  ffmpeg: FfmpegStatus | null;
  /** Paths waiting to be opened in the editor, in drop order. */
  inbox: string[];
  draft: Draft | null;
  /** Path currently being probed, if any. */
  opening: string | null;
  /** Recipe picked before choosing files; applied to the next files. */
  recipe: RecipeId | null;
  jobs: JobEntry[];
  history: HistoryItem[];
  /** History outputs that are no longer where we wrote them (moved or deleted). */
  missing: string[];
  /** Starting settings for freshly opened files. */
  settings: Defaults;
  /** Recipes the user saved from the editor. */
  recipes: Recipe[];
  /** Waiting files being queued with one set of settings. */
  batch: { done: number; total: number } | null;
  toasts: Toast[];
};

const HISTORY_KEY = "catconvert.history.v1";
const SETTINGS_KEY = "catconvert.settings.v1";
const RECIPES_KEY = "catconvert.recipes.v1";
const HISTORY_LIMIT = 50;

// Storage can be unavailable (private mode, quota). Everything kept here is a
// convenience, so failures fall back to defaults silently.
function load(key: string): unknown {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // See above.
  }
}

function loadHistory(): HistoryItem[] {
  const parsed = load(HISTORY_KEY);
  return Array.isArray(parsed) ? (parsed as HistoryItem[]) : [];
}

function saveHistory(items: HistoryItem[]) {
  save(HISTORY_KEY, items);
}

function loadSettings(): Defaults {
  const parsed = load(SETTINGS_KEY) as Partial<Defaults> | null;
  if (!parsed || typeof parsed !== "object") return structuredClone(DEFAULTS);
  return {
    ...DEFAULTS,
    ...parsed,
    formats: { ...DEFAULTS.formats, ...parsed.formats },
  };
}

function loadRecipes(): Recipe[] {
  const parsed = load(RECIPES_KEY);
  return Array.isArray(parsed)
    ? (parsed as Recipe[]).filter((r) => r && typeof r.id === "string" && r.options?.format)
    : [];
}

const [state, setState] = createStore<State>({
  screen: "convert",
  appVersion: null,
  update: { status: "idle" },
  ffmpeg: null,
  inbox: [],
  draft: null,
  opening: null,
  recipe: null,
  jobs: [],
  history: loadHistory(),
  missing: [],
  settings: loadSettings(),
  recipes: loadRecipes(),
  batch: null,
  toasts: [],
});

export { state };

/** Latest event for jobs the UI has not registered yet (events can beat `startJob`'s reply). */
const early = new Map<JobId, JobProgress>();
let nextToast = 1;

// ---------------------------------------------------------------------------
// Navigation and toasts
// ---------------------------------------------------------------------------

export function navigate(screen: Screen) {
  setState("screen", screen);
}

export function toast(message: string, options: Partial<Omit<Toast, "id" | "message">> = {}) {
  const id = nextToast++;
  setState("toasts", (list) => [...list.slice(-2), { id, message, tone: "info", ...options }]);
  setTimeout(() => dismissToast(id), options.tone === "error" ? 6000 : 3800);
}

export function dismissToast(id: number) {
  setState("toasts", (list) => list.filter((t) => t.id !== id));
}

// ---------------------------------------------------------------------------
// ffmpeg
// ---------------------------------------------------------------------------

export async function refreshFfmpeg(relocate = false) {
  try {
    const status = relocate ? await api.relocateFfmpeg() : await api.ffmpegStatus();
    setState("ffmpeg", status);
    return status;
  } catch (error) {
    const status: FfmpegStatus = {
      found: false,
      bundled: false,
      version: null,
      path: null,
      error: String(error),
      formats: [],
      libjxl: null,
      jxl: api.NO_JXL,
    };
    setState("ffmpeg", status);
    return status;
  }
}

export function isAvailable(format: OutputFormat): boolean {
  return state.ffmpeg?.formats.find((f) => f.format === format)?.available ?? false;
}

// ---------------------------------------------------------------------------
// Updates
// ---------------------------------------------------------------------------

// `setState({ update })` replaces the union value; `setState("update", ...)`
// would merge it and leave keys of the previous variant behind.
function setUpdate(update: UpdateState) {
  setState({ update });
}

export async function loadAppVersion() {
  try {
    setState("appVersion", await api.appVersion());
  } catch {
    // Only used for display.
  }
}

/**
 * Look for a newer release. The quiet check at launch only speaks up when an
 * update exists: being offline or rate limited is not worth a toast.
 */
export async function checkForUpdates({ quiet = false } = {}) {
  const busy = state.update.status;
  if (busy === "checking" || busy === "downloading" || busy === "ready") return;
  setUpdate({ status: "checking" });
  try {
    const info = await api.checkForUpdate();
    if (!info) {
      setUpdate({ status: "current" });
      return;
    }
    setUpdate({ status: "available", info });
    if (quiet) {
      toast(`CatConvert ${info.version} is out`, {
        action: { label: "Update", run: () => navigate("settings") },
      });
    }
  } catch (error) {
    setUpdate(
      quiet ? { status: "idle" } : { status: "error", message: errorText(error), info: null },
    );
  }
}

/** Download and install the update found by the last check. */
export async function installUpdate() {
  const current = state.update;
  const info = current.status === "available" || current.status === "error" ? current.info : null;
  if (!info) return;
  if (activeCount() > 0) {
    toast("Let your conversions finish first", { tone: "error" });
    return;
  }
  setUpdate({ status: "downloading", info, progress: { downloaded: 0, total: null } });
  try {
    await api.installUpdate((progress) => setUpdate({ status: "downloading", info, progress }));
    setUpdate({ status: "ready", info });
  } catch (error) {
    setUpdate({ status: "error", message: errorText(error), info });
  }
}

/** Restart into the installed update. */
export async function restartApp() {
  try {
    await api.relaunch();
  } catch (error) {
    toast(`Couldn't restart: ${errorText(error)}`, { tone: "error" });
  }
}

// ---------------------------------------------------------------------------
// Opening files
// ---------------------------------------------------------------------------

/** Queue dropped or picked files for editing; opens the first right away. */
export function openPaths(paths: string[]) {
  if (paths.length === 0) return;
  setState("inbox", (inbox) => [...inbox, ...paths.filter((p) => !inbox.includes(p))]);
  if (!state.draft && !state.opening) void openNext();
  else if (state.draft) toast(`${paths.length} more waiting their turn`);
}

/** Pick files with the native dialog, optionally applying a recipe to them. */
export async function chooseFiles(recipe: RecipeId | null = null) {
  try {
    const paths = await api.pickFiles();
    if (paths.length === 0) return;
    if (recipe) setState("recipe", recipe);
    openPaths(paths);
  } catch (error) {
    toast(`Couldn't open the file picker: ${String(error)}`, { tone: "error" });
  }
}

/** Built-in recipes followed by the user's own. */
export function allRecipes(): Recipe[] {
  return [...RECIPES, ...state.recipes];
}

function currentRecipe(): Recipe | null {
  return allRecipes().find((r) => r.id === state.recipe) ?? null;
}

/** Probe the next inbox file and open it in the editor. */
export async function openNext(): Promise<void> {
  const path = state.inbox[0];
  if (path === undefined) {
    batch(() => {
      setState({ draft: null, opening: null, recipe: null });
      if (state.screen === "edit") navigate(state.jobs.length > 0 ? "queue" : "convert");
    });
    return;
  }
  batch(() => {
    setState({ opening: path, inbox: state.inbox.slice(1) });
    navigate("edit");
  });
  try {
    const info = await api.probeFile(path);
    if (info.kind === "unknown") throw new Error("That doesn't look like a photo, video or song.");
    setState({
      draft: { info, options: initialOptions(info, isAvailable, currentRecipe(), state.settings) },
      opening: null,
    });
  } catch (error) {
    setState("opening", null);
    toast(`${baseName(path)}: ${errorText(error)}`, { tone: "error" });
    return openNext();
  }
}

/** Leave the current file without converting it. */
export function skipDraft() {
  setState("draft", null);
  void openNext();
}

/** Drop the current file and everything waiting. */
export function closeEditor() {
  batch(() => {
    setState({ draft: null, inbox: [], opening: null, recipe: null });
    navigate("convert");
  });
}

export function updateOptions(patch: Partial<ConvertOptions>) {
  if (!state.draft) return;
  setState("draft", "options", (options) => ({ ...options, ...patch }));
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/** Queue the current draft and move on to the next file (or the queue). */
export async function convertDraft() {
  const draft = state.draft;
  if (!draft) return;
  const options = { ...draft.options };
  try {
    const ticket = await api.startJob(draft.info.path, options);
    addJob(ticket, options);
    if (state.inbox.length > 0) {
      toast(`${draft.info.fileName} is in the queue`, {
        action: { label: "View", run: () => navigate("queue") },
      });
      setState("draft", null);
      await openNext();
    } else {
      batch(() => {
        setState({ draft: null, recipe: null });
        navigate("queue");
      });
    }
  } catch (error) {
    toast(errorText(error), { tone: "error" });
  }
}

/**
 * Queue the current file, then every waiting file with the same settings
 * (minus the trim, which belongs to one file). Files the settings don't fit,
 * such as a song when making MP4s, go back to the editor afterwards.
 */
export async function convertAll() {
  const draft = state.draft;
  if (!draft) return;
  const options = {
    ...draft.options,
    jxl: { ...draft.options.jxl },
    sound: { ...draft.options.sound },
  };
  const paths = [...state.inbox];
  try {
    addJob(await api.startJob(draft.info.path, options), options);
  } catch (error) {
    toast(errorText(error), { tone: "error" });
    return;
  }
  batch(() => {
    setState({ draft: null, inbox: [], recipe: null, batch: { done: 0, total: paths.length } });
    navigate("queue");
  });

  const leftovers: string[] = [];
  for (const path of paths) {
    try {
      const info = await api.probeFile(path);
      if (info.kind === "unknown")
        throw new Error("That doesn't look like a photo, video or song.");
      const fitted = fitOptions(info, options, isAvailable);
      if (fitted) addJob(await api.startJob(path, fitted), fitted);
      else leftovers.push(path);
    } catch (error) {
      toast(`${baseName(path)}: ${errorText(error)}`, { tone: "error" });
    }
    setState("batch", (progress) => progress && { ...progress, done: progress.done + 1 });
  }
  setState("batch", null);

  if (leftovers.length > 0) {
    const n = leftovers.length;
    toast(
      n === 1
        ? `${baseName(leftovers[0] ?? "")} needs its own settings`
        : `${n} files need their own settings`,
    );
    openPaths(leftovers);
  }
}

function addJob(ticket: api.JobTicket, options: ConvertOptions) {
  const entry: JobEntry = {
    id: ticket.id,
    name: ticket.input.fileName,
    input: ticket.input,
    output: ticket.output,
    options,
    command: ticket.command,
    stage: "queued",
    progress: 0,
    speed: null,
    outputBytes: null,
    error: null,
    submittedAt: Date.now(),
    finishedAt: null,
  };
  setState("jobs", (jobs) => [...jobs, entry]);
  const pending = early.get(ticket.id);
  if (pending) {
    early.delete(ticket.id);
    applyProgress(pending);
  }
}

/** Apply a `job-progress` event from the backend. */
export function applyProgress(event: JobProgress) {
  const index = state.jobs.findIndex((job) => job.id === event.jobId);
  if (index < 0) {
    early.set(event.jobId, event);
    return;
  }
  const job = state.jobs[index];
  if (!job || isFinal(job.stage)) return;

  const finished = isFinal(event.stage);
  setState(
    "jobs",
    index,
    produce((draft) => {
      draft.stage = event.stage;
      draft.progress = event.progress;
      draft.speed = event.speed;
      draft.outputBytes = event.outputBytes;
      draft.error = event.error;
      if (finished) draft.finishedAt = Date.now();
    }),
  );

  if (event.stage === "done") {
    const item: HistoryItem = {
      id: `${job.submittedAt}-${job.id}`,
      name: baseName(job.output),
      output: job.output,
      format: job.options.format,
      kind: job.input.kind,
      inputBytes: job.input.sizeBytes,
      outputBytes: event.outputBytes ?? 0,
      finishedAt: Date.now(),
    };
    setState("history", (items) => {
      const next = [item, ...items].slice(0, HISTORY_LIMIT);
      saveHistory(next);
      return next;
    });
  } else if (event.stage === "failed" && state.screen !== "queue") {
    toast(`${job.name} didn't convert`, {
      tone: "error",
      action: { label: "See why", run: () => navigate("queue") },
    });
  }
}

export function isFinal(stage: JobStage) {
  return stage === "done" || stage === "failed" || stage === "cancelled";
}

export async function cancelJob(id: JobId) {
  try {
    await api.cancelJob(id);
  } catch (error) {
    toast(errorText(error), { tone: "error" });
  }
}

/** Reopen a finished or failed job's file in the editor with the same settings. */
export function tweakJob(id: JobId) {
  const job = state.jobs.find((j) => j.id === id);
  if (!job) return;
  batch(() => {
    setState({ draft: { info: job.input, options: { ...job.options } }, opening: null });
    navigate("edit");
  });
}

export async function retryJob(id: JobId) {
  const job = state.jobs.find((j) => j.id === id);
  if (!job) return;
  try {
    const options = { ...job.options };
    const ticket = await api.startJob(job.input.path, options);
    batch(() => {
      removeJob(id);
      addJob(ticket, options);
    });
  } catch (error) {
    toast(errorText(error), { tone: "error" });
  }
}

export function removeJob(id: JobId) {
  setState("jobs", (jobs) => jobs.filter((j) => j.id !== id));
}

export function clearFinished() {
  setState("jobs", (jobs) => jobs.filter((j) => !isFinal(j.stage)));
}

export function clearHistory() {
  batch(() => {
    setState({ history: [], missing: [] });
    saveHistory([]);
  });
}

/** Note which history outputs have been moved or deleted since. */
export async function checkHistory() {
  try {
    const missing = await api.missingFiles(state.history.map((item) => item.output));
    setState("missing", missing);
  } catch {
    // Best effort: rows simply stay as they are.
  }
}

/** Drop history entries whose files are gone. */
export function forgetMissing() {
  const gone = new Set(state.missing);
  batch(() => {
    setState("history", (items) => {
      const next = items.filter((item) => !gone.has(item.output));
      saveHistory(next);
      return next;
    });
    setState("missing", []);
  });
}

export async function reveal(path: string) {
  try {
    await api.revealInFolder(path);
  } catch (error) {
    toast(`Couldn't show the file: ${errorText(error)}`, { tone: "error" });
    void checkHistory();
  }
}

// ---------------------------------------------------------------------------
// Settings and recipes
// ---------------------------------------------------------------------------

export function updateSettings(patch: Partial<Defaults>) {
  setState("settings", (settings) => {
    const next = { ...settings, ...patch, formats: { ...settings.formats, ...patch.formats } };
    save(SETTINGS_KEY, next);
    return next;
  });
}

function setRecipes(recipes: Recipe[]) {
  setState("recipes", recipes);
  save(RECIPES_KEY, recipes);
}

/** Save the editor's current settings as a recipe. */
export function saveRecipe(title: string) {
  const draft = state.draft;
  const name = title.trim();
  if (!draft || !name) return;
  const id = `custom-${Date.now().toString(36)}-${state.recipes.length}`;
  setRecipes([...state.recipes, recipeFrom(id, name, draft.info, { ...draft.options })]);
  toast(`Saved “${name}” to your recipes`, {
    action: { label: "View", run: () => navigate("recipes") },
  });
}

export function removeRecipe(id: RecipeId) {
  const index = state.recipes.findIndex((r) => r.id === id);
  const recipe = state.recipes[index];
  if (!recipe) return;
  setRecipes(state.recipes.filter((r) => r.id !== id));
  toast(`Removed “${recipe.title}”`, {
    action: {
      label: "Undo",
      run: () => {
        const restored = [...state.recipes];
        restored.splice(Math.min(index, restored.length), 0, recipe);
        setRecipes(restored);
      },
    },
  });
}

/** Jobs still waiting or running, for the Queue badge. */
export function activeCount() {
  return state.jobs.filter((j) => !isFinal(j.stage)).length;
}

export function errorText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Test helper: reset everything to a fresh state. */
export function resetStore() {
  early.clear();
  setState({
    screen: "convert",
    appVersion: null,
    update: { status: "idle" },
    ffmpeg: null,
    inbox: [],
    draft: null,
    opening: null,
    recipe: null,
    jobs: [],
    history: [],
    missing: [],
    settings: structuredClone(DEFAULTS),
    recipes: [],
    batch: null,
    toasts: [],
  });
}
