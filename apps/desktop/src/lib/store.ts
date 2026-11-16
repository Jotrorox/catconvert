/**
 * App state: which screen is showing, the file being edited, files waiting
 * their turn, the conversion queue and the history of finished conversions.
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
} from "./api";
import * as api from "./api";
import { baseName } from "./format";
import { initialOptions, RECIPES, type Recipe, type RecipeId } from "./formats";

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

type State = {
  screen: Screen;
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
  toasts: Toast[];
};

const HISTORY_KEY = "catconvert.history.v1";
const HISTORY_LIMIT = 50;

function loadHistory(): HistoryItem[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? (parsed as HistoryItem[]) : [];
  } catch {
    return [];
  }
}

function saveHistory(items: HistoryItem[]) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(items));
  } catch {
    // Storage can be unavailable (private mode, quota); history is a convenience.
  }
}

const [state, setState] = createStore<State>({
  screen: "convert",
  ffmpeg: null,
  inbox: [],
  draft: null,
  opening: null,
  recipe: null,
  jobs: [],
  history: loadHistory(),
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
      version: null,
      path: null,
      error: String(error),
      formats: [],
    };
    setState("ffmpeg", status);
    return status;
  }
}

export function isAvailable(format: OutputFormat): boolean {
  return state.ffmpeg?.formats.find((f) => f.format === format)?.available ?? false;
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

function currentRecipe(): Recipe | null {
  return RECIPES.find((r) => r.id === state.recipe) ?? null;
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
      draft: { info, options: initialOptions(info, isAvailable, currentRecipe()) },
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
  setState("history", []);
  saveHistory([]);
}

export async function reveal(path: string) {
  try {
    await api.revealInFolder(path);
  } catch (error) {
    toast(`Couldn't show the file: ${errorText(error)}`, { tone: "error" });
  }
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
    ffmpeg: null,
    inbox: [],
    draft: null,
    opening: null,
    recipe: null,
    jobs: [],
    history: [],
    toasts: [],
  });
}
