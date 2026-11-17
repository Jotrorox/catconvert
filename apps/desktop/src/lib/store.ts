/**
 * App state: which screen is showing, the file (or pictures for an
 * animation) being edited, files waiting their turn, the conversion queue,
 * the history of finished conversions, and the user's starting settings and
 * saved recipes.
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
  QuickActionStatus,
  SequenceOptions,
  UpdateInfo,
  VideoCodec,
} from "./api";
import * as api from "./api";
import { baseName } from "./format";
import {
  codecFor,
  DEFAULT_SEQUENCE,
  DEFAULTS,
  type Defaults,
  fitOptions,
  initialOptions,
  looksLikePicture,
  metadataFrom,
  naturalOrder,
  RECIPES,
  type Recipe,
  type RecipeId,
  recipeFrom,
  sequenceOptions,
} from "./formats";

export type Screen = "convert" | "edit" | "queue" | "recipes" | "history" | "settings";

export type Draft = {
  info: MediaInfo;
  options: ConvertOptions;
  /**
   * The command as edited by hand, run instead of the one the options make.
   * `null` while the generated command applies.
   */
  command: string | null;
};

/** Pictures being made into one animation, in the order they play. */
export type SequenceDraft = {
  pictures: MediaInfo[];
  options: ConvertOptions;
  sequence: SequenceOptions;
};

export type JobEntry = {
  id: JobId;
  name: string;
  /** The file, or the first picture of an animation. */
  input: MediaInfo;
  /** Every input: just `input`, or an animation's pictures in order. */
  inputs: MediaInfo[];
  output: string;
  options: ConvertOptions;
  /** Set for an animation made from pictures. */
  sequence: SequenceOptions | null;
  command: string;
  /** `command` was edited by hand rather than made from `options`. */
  custom: boolean;
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
  /** How many pictures an animation was made from. */
  pictures?: number;
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
  /** Dropped pictures waiting for "Convert each" or "Make an animation". */
  choice: string[] | null;
  draft: Draft | null;
  /** Pictures being made into one animation. */
  sequence: SequenceDraft | null;
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
  /** The Finder Quick Action, once Settings has looked. */
  quickAction: QuickActionStatus | null;
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
  const parsed = load(SETTINGS_KEY) as (Partial<Defaults> & { keepMetadata?: unknown }) | null;
  if (!parsed || typeof parsed !== "object") return structuredClone(DEFAULTS);
  const { keepMetadata: _old, ...rest } = parsed;
  return {
    ...DEFAULTS,
    ...rest,
    formats: { ...DEFAULTS.formats, ...parsed.formats },
    metadata: metadataFrom(parsed) ?? DEFAULTS.metadata,
  };
}

function loadRecipes(): Recipe[] {
  const parsed = load(RECIPES_KEY);
  if (!Array.isArray(parsed)) return [];
  return (parsed as Recipe[])
    .filter((r) => r && typeof r.id === "string" && r.options?.format)
    .map((recipe) => {
      // Recipes saved before the three-way choice stored `keepMetadata`.
      const { keepMetadata: _old, ...options } = recipe.options as Recipe["options"] & {
        keepMetadata?: unknown;
      };
      const metadata = metadataFrom(recipe.options);
      return { ...recipe, options: metadata ? { ...options, metadata } : options };
    });
}

const [state, setState] = createStore<State>({
  screen: "convert",
  appVersion: null,
  update: { status: "idle" },
  ffmpeg: null,
  inbox: [],
  choice: null,
  draft: null,
  sequence: null,
  opening: null,
  recipe: null,
  jobs: [],
  history: loadHistory(),
  missing: [],
  settings: loadSettings(),
  recipes: loadRecipes(),
  batch: null,
  quickAction: null,
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
      videoCodecs: [],
      hevcHardware: false,
      readsAv1: false,
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

/** Whether ffmpeg can write video in `codec`. */
export function isCodecAvailable(codec: VideoCodec): boolean {
  return state.ffmpeg?.videoCodecs?.includes(codec) ?? false;
}

/** Whether H.265 can be encoded in hardware (VideoToolbox on Apple silicon). */
export function canEncodeHevcInHardware(): boolean {
  return state.ffmpeg?.hevcHardware ?? false;
}

/** Whether ffmpeg can read the picture of `info`: AV1 needs dav1d or libaom. */
export function canRead(info: MediaInfo): boolean {
  return info.videoCodec !== "av1" || state.ffmpeg?.readsAv1 !== false;
}

/**
 * `patch` applied to `options`. A new format keeps the codec choice when it
 * can hold that codec (AV1 MP4 → AV1 WebM) and falls back to its usual one
 * otherwise.
 */
function patched(options: ConvertOptions, patch: Partial<ConvertOptions>): ConvertOptions {
  const next = { ...options, ...patch };
  if (patch.format !== undefined && patch.videoCodec === undefined) {
    next.videoCodec = codecFor(patch.format, options.videoCodec ?? null);
  }
  return next;
}

/** Whether pictures or a video can become an animation in `format`. */
export function isAnimated(format: OutputFormat): boolean {
  return state.ffmpeg?.formats.find((f) => f.format === format)?.animated ?? false;
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

/**
 * Queue dropped or picked files for editing; opens the first right away.
 * Two or more pictures dropped with nothing open first ask whether to
 * convert each or make one animation of them.
 */
export function openPaths(paths: string[]) {
  const fresh = [...new Set(paths)].filter((p) => !state.inbox.includes(p));
  if (fresh.length === 0) return;
  const pictures = fresh.every(looksLikePicture);
  if (state.choice) {
    if (pictures) {
      setState("choice", (choice) => [
        ...(choice ?? []),
        ...fresh.filter((p) => !choice?.includes(p)),
      ]);
      return;
    }
  } else if (pictures && fresh.length >= 2 && !state.recipe && !busyEditing()) {
    batch(() => {
      setState("choice", fresh);
      navigate("edit");
    });
    return;
  }
  setState("inbox", (inbox) => [...inbox, ...fresh]);
  if (busyEditing() || state.choice) toast(`${fresh.length} more waiting their turn`);
  else void openNext();
}

/** Whether the editor already holds something. */
function busyEditing(): boolean {
  return state.draft !== null || state.sequence !== null || state.opening !== null;
}

/** "Convert each": the dropped pictures wait their turn like any other files. */
export function convertEach() {
  const paths = state.choice ?? [];
  setState({ choice: null, inbox: [...paths, ...state.inbox.filter((p) => !paths.includes(p))] });
  void openNext();
}

/** Probe `paths` a few at a time, keeping the order. */
async function probeAll(paths: string[], limit = 4) {
  const results: PromiseSettledResult<MediaInfo>[] = new Array(paths.length);
  let next = 0;
  const worker = async () => {
    while (next < paths.length) {
      const i = next++;
      const path = paths[i] ?? "";
      results[i] = await api.probeFile(path).then(
        (value) => ({ status: "fulfilled", value }) as const,
        (reason: unknown) => ({ status: "rejected", reason }) as const,
      );
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, paths.length) }, worker));
  return results;
}

/**
 * "Make an animation": probe the dropped pictures and open them in the
 * sequence editor, in natural filename order.
 */
export async function makeAnimation() {
  const paths = state.choice ?? [];
  batch(() => {
    setState({ choice: null, opening: `${paths.length} pictures` });
    navigate("edit");
  });
  const results = await probeAll(paths);
  const pictures: MediaInfo[] = [];
  const left: string[] = [];
  results.forEach((result, i) => {
    if (result.status === "fulfilled" && result.value.kind === "image") pictures.push(result.value);
    else left.push(paths[i] ?? "");
  });
  if (left.length > 0) {
    toast(
      left.length === 1
        ? `${baseName(left[0] ?? "")} isn't a still picture, so it's left out`
        : `${left.length} files aren't still pictures, so they're left out`,
    );
  }
  if (pictures.length < 2) {
    setState("opening", null);
    toast("An animation needs at least two pictures", { tone: "error" });
    setState("inbox", (inbox) => [...pictures.map((p) => p.path), ...inbox]);
    return openNext();
  }
  pictures.sort(naturalOrder);
  setState({
    opening: null,
    sequence: {
      pictures,
      options: sequenceOptions(isAnimated, state.settings),
      sequence: { ...DEFAULT_SEQUENCE },
    },
  });
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
    const recipe = currentRecipe();
    const options = initialOptions(info, isAvailable, recipe, state.settings);
    setState({
      draft: { info, options, command: await recipeCommand(recipe, info, options) },
      opening: null,
    });
  } catch (error) {
    setState("opening", null);
    toast(`${baseName(path)}: ${errorText(error)}`, { tone: "error" });
    return openNext();
  }
}

/**
 * A recipe's saved command filled in for `info`, if it has one and its format
 * fits the file. Left out (with a word why) when the paths can't be filled in.
 */
async function recipeCommand(
  recipe: Recipe | null,
  info: MediaInfo,
  options: ConvertOptions,
): Promise<string | null> {
  if (!recipe?.command || recipe.options.format !== options.format) return null;
  try {
    return await api.fillCommand(info, options, recipe.command);
  } catch (error) {
    toast(`${info.fileName}: couldn't use the recipe's command: ${errorText(error)}`, {
      tone: "error",
    });
    return null;
  }
}

/** Leave the current file without converting it. */
export function skipDraft() {
  setState("draft", null);
  void openNext();
}

/** Drop the current file (or animation) and everything waiting. */
export function closeEditor() {
  batch(() => {
    setState({
      draft: null,
      sequence: null,
      choice: null,
      inbox: [],
      opening: null,
      recipe: null,
    });
    navigate("convert");
  });
}

export function updateOptions(patch: Partial<ConvertOptions>) {
  if (!state.draft) return;
  setState("draft", "options", (options) => patched(options, patch));
}

/**
 * Run `command` instead of the generated command for the current file, or go
 * back to the generated one with `null`.
 */
export function setCustomCommand(command: string | null) {
  if (!state.draft) return;
  setState("draft", "command", command);
}

// ---------------------------------------------------------------------------
// Animations from pictures
// ---------------------------------------------------------------------------

/** Change the format, size, turn, direction or quality of the animation being made. */
export function updateSequenceOptions(patch: Partial<ConvertOptions>) {
  if (!state.sequence) return;
  setState("sequence", "options", (options) => patched(options, patch));
}

/** Change the frame time or fit of the animation being made. */
export function updateSequence(patch: Partial<SequenceOptions>) {
  if (!state.sequence) return;
  setState("sequence", "sequence", (sequence) => ({ ...sequence, ...patch }));
}

/** Move the picture at `from` to `to` in the filmstrip. */
export function movePicture(from: number, to: number) {
  const list = state.sequence?.pictures;
  if (!list || from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) {
    return;
  }
  setState("sequence", "pictures", (pictures) => {
    const next = [...pictures];
    const [moved] = next.splice(from, 1);
    if (moved) next.splice(to, 0, moved);
    return next;
  });
}

/** Leave a picture out of the animation; at least two stay. */
export function removePicture(index: number) {
  const list = state.sequence?.pictures;
  if (!list) return;
  if (list.length <= 2) {
    toast("An animation needs at least two pictures", { tone: "error" });
    return;
  }
  setState("sequence", "pictures", (pictures) => pictures.filter((_, i) => i !== index));
}

/** Put the pictures back in natural filename order. */
export function sortPictures() {
  setState("sequence", "pictures", (pictures) => [...pictures].sort(naturalOrder));
}

/** Queue the animation and move on to the next waiting file (or the queue). */
export async function convertSequence() {
  const draft = state.sequence;
  if (!draft) return;
  const options = { ...draft.options, jxl: { ...draft.options.jxl } };
  const sequence = { ...draft.sequence };
  try {
    const ticket = await api.startSequence(
      draft.pictures.map((p) => p.path),
      options,
      sequence,
    );
    addJob(ticket, options, sequence);
    setState("sequence", null);
    if (state.inbox.length > 0) {
      toast("The animation is in the queue", {
        action: { label: "View", run: () => navigate("queue") },
      });
      await openNext();
    } else {
      batch(() => {
        setState("recipe", null);
        navigate("queue");
      });
    }
  } catch (error) {
    toast(errorText(error), { tone: "error" });
  }
}

/**
 * An animation job's pictures that are still where they were, saying which
 * were moved or deleted since. `null` (after saying so) when fewer than two
 * are left.
 */
async function picturesLeft(job: JobEntry): Promise<MediaInfo[] | null> {
  let missing: string[] = [];
  try {
    missing = await api.missingFiles(job.inputs.map((p) => p.path));
  } catch {
    // Best effort: the backend reports a missing picture when it is queued.
  }
  const left = job.inputs.filter((p) => !missing.includes(p.path));
  if (left.length < 2) {
    toast(`Only ${left.length} of the ${job.inputs.length} pictures are still there`, {
      tone: "error",
    });
    return null;
  }
  if (missing.length > 0) {
    const gone = missing.length === 1 ? "1 picture was" : `${missing.length} pictures were`;
    toast(`${gone} moved or deleted, so it's made from the other ${left.length}`);
  }
  return left;
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/** Queue the current draft and move on to the next file (or the queue). */
export async function convertDraft() {
  const draft = state.draft;
  if (!draft) return;
  const options = { ...draft.options };
  const custom = draft.command;
  try {
    const ticket =
      custom === null
        ? await api.startJob(draft.info.path, options)
        : await api.startCustomJob(draft.info.path, options, custom);
    addJob(ticket, options, null, custom !== null);
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
 *
 * A command edited by hand runs for every file, with each file's paths.
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
  const custom = draft.command;
  let template: string | null = null;
  try {
    if (custom === null) {
      addJob(await api.startJob(draft.info.path, options), options);
    } else {
      addJob(await api.startCustomJob(draft.info.path, options, custom), options, null, true);
      template = await api.commandTemplate(draft.info.path, custom);
    }
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
      // The command only fits files that keep its format.
      if (!fitted || (template !== null && fitted.format !== options.format)) leftovers.push(path);
      else if (template !== null)
        addJob(await api.startCustomJob(path, fitted, template), fitted, null, true);
      else addJob(await api.startJob(path, fitted), fitted);
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

function addJob(
  ticket: api.JobTicket,
  options: ConvertOptions,
  sequence: SequenceOptions | null = null,
  custom = false,
) {
  const input = ticket.inputs[0];
  if (!input) return;
  const entry: JobEntry = {
    id: ticket.id,
    name: sequence ? `${ticket.inputs.length} pictures` : input.fileName,
    input,
    inputs: ticket.inputs,
    output: ticket.output,
    options,
    sequence,
    command: ticket.command,
    custom,
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
    remember({
      id: `${job.submittedAt}-${job.id}`,
      name: baseName(job.output),
      output: job.output,
      format: job.options.format,
      kind: job.input.kind,
      inputBytes: inputBytes(job),
      outputBytes: event.outputBytes ?? 0,
      finishedAt: Date.now(),
      ...(job.sequence ? { pictures: job.inputs.length } : {}),
    });
  } else if (event.stage === "failed" && state.screen !== "queue") {
    toast(`${job.name} didn't convert`, {
      tone: "error",
      action: { label: "See why", run: () => navigate("queue") },
    });
  }
}

/** Size of everything a job reads. */
export function inputBytes(job: JobEntry): number {
  return job.inputs.reduce((sum, info) => sum + info.sizeBytes, 0);
}

function remember(item: HistoryItem) {
  setState("history", (items) => {
    const next = [item, ...items].slice(0, HISTORY_LIMIT);
    saveHistory(next);
    return next;
  });
}

/**
 * Save the frame at `at` seconds of the open video as a PNG next to it, with
 * the editor's turn, mirror and size. Runs right away rather than queueing.
 */
export async function saveFrame(at: number) {
  const draft = state.draft;
  if (!draft) return;
  const { info } = draft;
  try {
    const saved = await api.saveFrame(info.path, { ...draft.options }, at);
    remember({
      id: `${Date.now()}-${saved.output}`,
      name: baseName(saved.output),
      output: saved.output,
      format: "png",
      kind: info.kind,
      inputBytes: info.sizeBytes,
      outputBytes: saved.outputBytes,
      finishedAt: Date.now(),
    });
    toast("Saved frame", {
      action: { label: "Show in folder", run: () => void reveal(saved.output) },
    });
  } catch (error) {
    toast(`Couldn't save the frame: ${errorText(error)}`, { tone: "error" });
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

/** Reopen a finished or failed job's file(s) in the editor with the same settings. */
export async function tweakJob(id: JobId) {
  const job = state.jobs.find((j) => j.id === id);
  if (!job) return;
  if (job.sequence) {
    const sequence = { ...job.sequence };
    const pictures = await picturesLeft(job);
    if (!pictures) return;
    batch(() => {
      setState({
        sequence: { pictures, options: { ...job.options }, sequence },
        draft: null,
        opening: null,
      });
      navigate("edit");
    });
    return;
  }
  batch(() => {
    setState({
      draft: {
        info: job.input,
        options: { ...job.options },
        command: job.custom ? job.command : null,
      },
      sequence: null,
      opening: null,
    });
    navigate("edit");
  });
}

export async function retryJob(id: JobId) {
  const job = state.jobs.find((j) => j.id === id);
  if (!job) return;
  try {
    const options = { ...job.options, jxl: { ...job.options.jxl } };
    const sequence = job.sequence ? { ...job.sequence } : null;
    let ticket: api.JobTicket;
    if (sequence) {
      const pictures = await picturesLeft(job);
      if (!pictures) return;
      ticket = await api.startSequence(
        pictures.map((p) => p.path),
        options,
        sequence,
      );
    } else if (job.custom) {
      // Run as it ran; the output moves to the next free name if need be.
      ticket = await api.startCustomJob(job.input.path, options, job.command);
    } else {
      ticket = await api.startJob(job.input.path, options);
    }
    batch(() => {
      removeJob(id);
      addJob(ticket, options, sequence, job.custom);
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

/** Drag a finished file into another app or folder; `icon` is the drag image. */
export async function dragOut(path: string, icon: string) {
  if (state.missing.includes(path)) return;
  try {
    await api.dragFiles([path], icon);
  } catch (error) {
    toast(`Couldn't drag ${baseName(path)}: ${errorText(error)}`, { tone: "error" });
    void checkHistory();
  }
}

// ---------------------------------------------------------------------------
// Finder Quick Action
// ---------------------------------------------------------------------------

export async function loadQuickAction() {
  try {
    setState("quickAction", await api.quickActionStatus());
  } catch {
    // Settings simply doesn't offer it.
  }
}

/** Add the Quick Action if it's missing, remove it otherwise. */
export async function toggleQuickAction() {
  const current = state.quickAction;
  if (!current?.supported) return;
  try {
    const next = await api.setQuickAction(!current.installed);
    setState("quickAction", next);
    toast(
      next.installed
        ? "Right-click files in Finder › Quick Actions › Convert with CatConvert"
        : "Removed from Finder's Quick Actions",
    );
  } catch (error) {
    toast(`Couldn't change the Quick Action: ${errorText(error)}`, { tone: "error" });
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

/**
 * Save the editor's current settings as a recipe, with the edited command if
 * there is one (its paths become placeholders for the files it's used on).
 */
export async function saveRecipe(title: string) {
  const draft = state.draft;
  const name = title.trim();
  if (!draft || !name) return;
  const { info, command } = draft;
  const options = { ...draft.options };
  let template: string | null = null;
  if (command !== null) {
    try {
      template = await api.commandTemplate(info.path, command);
    } catch (error) {
      toast(`Couldn't save the command: ${errorText(error)}`, { tone: "error" });
      return;
    }
  }
  const id = `custom-${Date.now().toString(36)}-${state.recipes.length}`;
  setRecipes([...state.recipes, recipeFrom(id, name, info, options, template)]);
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

/** Test helper: reset everything to a fresh start, reading storage as at launch. */
export function resetStore() {
  early.clear();
  setState({
    screen: "convert",
    appVersion: null,
    update: { status: "idle" },
    ffmpeg: null,
    inbox: [],
    choice: null,
    draft: null,
    sequence: null,
    opening: null,
    recipe: null,
    jobs: [],
    history: loadHistory(),
    missing: [],
    settings: loadSettings(),
    recipes: loadRecipes(),
    batch: null,
    quickAction: null,
    toasts: [],
  });
}
