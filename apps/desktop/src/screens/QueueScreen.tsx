import { type Component, createMemo, For, Show } from "solid-js";
import { CatPeek } from "../components/Cat";
import { Icon } from "../components/Icon";
import { PlayButton } from "../components/PlayButton";
import { dragIcon, startDragOnMove } from "../lib/drag";
import { baseName, formatBytes, formatClock } from "../lib/format";
import { formatKind, label, playbackOrder } from "../lib/formats";
import {
  cancelJob,
  chooseFiles,
  clearFinished,
  dragOut,
  inputBytes,
  isFinal,
  type JobEntry,
  removeJob,
  retryJob,
  reveal,
  state,
  tweakJob,
} from "../lib/store";

/** Finished files can be dragged straight into another app. */
function dragDone(event: PointerEvent, job: JobEntry) {
  if (job.stage !== "done") return;
  startDragOnMove(
    event,
    () => void dragOut(job.output, dragIcon(job.options.format, job.input.kind)),
  );
}

function route(job: JobEntry) {
  const from = job.sequence
    ? `${job.inputs.length} PICTURES`
    : (job.input.extension ?? "?").toUpperCase();
  return `${from} → ${label(job.options.format)}`;
}

/** Length of what the job writes, in seconds. */
function outputLength(job: JobEntry): number {
  if (job.sequence) {
    return playbackOrder(job.inputs.length, job.options.direction).length * job.sequence.frameSecs;
  }
  const o = job.options;
  return (o.trimEnd ?? job.input.durationSecs ?? 0) - (o.trimStart ?? 0);
}

/** Seconds of output still to encode, divided by the current speed. */
function timeLeft(job: JobEntry): string | null {
  const length = outputLength(job);
  if (!job.speed || job.speed <= 0 || length <= 0 || job.progress <= 0) return null;
  return formatClock(((1 - job.progress) * length) / job.speed);
}

const ActiveJob: Component<{ job: JobEntry; compact: boolean }> = (props) => {
  const pct = () => Math.round(props.job.progress * 100);
  return (
    <section class="hero-job" aria-label="Converting now">
      <div class="hero-cat">
        <CatPeek sleepy />
        <span class="zzz" aria-hidden="true">
          z<sup>z</sup>
        </span>
      </div>
      <div class="hero-body">
        <div class="hero-top">
          <div class="hero-heading">
            <Show when={!props.compact}>
              <p class="kicker">{props.job.stage === "running" ? "Kneading" : "Warming up"}</p>
            </Show>
            <h2 class="hero-name">
              {props.job.name} → {label(props.job.options.format)}
            </h2>
          </div>
          <p class="display hero-pct">
            {pct()}
            <span class="hero-pct-sign">%</span>
          </p>
        </div>
        <div
          class="progress"
          role="progressbar"
          aria-label={`Converting ${props.job.name}`}
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={pct()}
        >
          <div class="progress-fill" style={{ width: `${Math.max(pct(), 4)}%` }}>
            <span class="progress-knob">
              <Icon name="paw" size={14} />
            </span>
          </div>
        </div>
        <p class="hero-stats">
          <Show when={props.job.speed}>
            {(speed) => (
              <span>
                <b>{speed().toFixed(1)}×</b> speed
              </span>
            )}
          </Show>
          <Show when={timeLeft(props.job)}>
            {(left) => (
              <span>
                <b>{left()}</b> left
              </span>
            )}
          </Show>
          <span class="hero-out" title={props.job.output}>
            into <b>{baseName(props.job.output)}</b>
          </span>
          <button
            type="button"
            class="link-btn hero-cancel"
            onClick={() => void cancelJob(props.job.id)}
          >
            Cancel
          </button>
        </p>
      </div>
    </section>
  );
};

export const QueueScreen: Component<{ compact: boolean }> = (props) => {
  const active = createMemo(() => state.jobs.find((j) => j.stage === "running"));
  const waiting = createMemo(() => state.jobs.filter((j) => j.stage === "queued"));
  const finished = createMemo(() =>
    state.jobs
      .filter((j) => isFinal(j.stage))
      .sort((a, b) => (b.finishedAt ?? 0) - (a.finishedAt ?? 0)),
  );
  const summary = () => {
    const left = waiting().length + (active() ? 1 : 0);
    if (left === 0) return finished().length > 0 ? "All done. Purrfect." : "Nothing cooking yet.";
    return `${left} to go`;
  };

  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1 class="display page-title">Queue</h1>
          <p class="subtitle">
            <Show when={state.batch} fallback={summary()}>
              {(progress) => `Adding files… ${progress().done} of ${progress().total}`}
            </Show>
          </p>
        </div>
        <div class="actions">
          <Show when={finished().length > 0}>
            <button type="button" class="btn btn-outline" onClick={() => clearFinished()}>
              Clear finished
            </button>
          </Show>
          <button type="button" class="btn btn-primary" onClick={() => void chooseFiles()}>
            <Icon name="plus" size={16} />
            Add files
          </button>
        </div>
      </header>

      <Show when={state.jobs.length === 0}>
        <section class="card placeholder">
          <CatPeek sleepy class="placeholder-cat" />
          <h2 class="label-strong">The kitty is napping</h2>
          <p class="item-meta">Drop a file anywhere to give it something to do.</p>
        </section>
      </Show>

      <Show when={active()}>{(job) => <ActiveJob job={job()} compact={props.compact} />}</Show>

      <Show when={waiting().length > 0}>
        <section>
          <h2 class="section-title">Up next</h2>
          <ul class="job-list">
            <For each={waiting()}>
              {(job, index) => (
                <li class="job-row">
                  <span class="tile" data-tone="lavender">
                    {index() + (active() ? 2 : 1)}
                  </span>
                  <div class="item-main">
                    <span class="item-name">{job.name}</span>
                    <span class="item-meta">{route(job)} · Waiting</span>
                  </div>
                  <button
                    type="button"
                    class="icon-btn plain"
                    aria-label={`Cancel ${job.name}`}
                    onClick={() => void cancelJob(job.id)}
                  >
                    <Icon name="close" size={18} />
                  </button>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

      <Show when={finished().length > 0}>
        <section>
          <h2 class="section-title">Finished</h2>
          <ul class="job-list">
            <For each={finished()}>
              {(job) => (
                <li
                  class="job-row pop"
                  classList={{ "drag-out": job.stage === "done" }}
                  data-stage={job.stage}
                  title={job.stage === "done" ? "Drag into another app or folder" : undefined}
                  onPointerDown={(event) => dragDone(event, job)}
                >
                  <Show
                    when={job.stage === "done"}
                    fallback={
                      <span class="tile round alert">{job.stage === "failed" ? "!" : "–"}</span>
                    }
                  >
                    <span class="tile round" data-tone="green">
                      <Icon name="check" size={20} />
                    </span>
                  </Show>
                  <div class="item-main">
                    <span class="item-name">
                      {job.stage === "done" ? baseName(job.output) : job.name}
                    </span>
                    <span class="item-meta">
                      <Show when={job.stage === "done"}>
                        {route(job)} · {formatBytes(inputBytes(job))} →{" "}
                        {formatBytes(job.outputBytes ?? 0)}
                      </Show>
                      <Show when={job.stage === "failed"}>
                        {job.error ?? "Something went wrong."}
                      </Show>
                      <Show when={job.stage === "cancelled"}>{route(job)} · Cancelled</Show>
                    </span>
                  </div>
                  <div class="row-actions">
                    <Show
                      when={
                        job.stage === "done" &&
                        formatKind(job.options.format, job.input.kind) === "audio"
                      }
                    >
                      <PlayButton
                        path={job.output}
                        name={baseName(job.output)}
                        format={job.options.format}
                      />
                    </Show>
                    <Show when={job.stage === "done"}>
                      <button
                        type="button"
                        class="btn btn-soft"
                        onClick={() => void reveal(job.output)}
                      >
                        <Icon name="folder" size={16} />
                        Show in folder
                      </button>
                    </Show>
                    <Show when={job.stage !== "done"}>
                      <button
                        type="button"
                        class="btn btn-outline"
                        onClick={() => tweakJob(job.id)}
                      >
                        Tweak
                      </button>
                      <button
                        type="button"
                        class="btn btn-outline"
                        onClick={() => void retryJob(job.id)}
                      >
                        <Icon name="refresh" size={16} />
                        Retry
                      </button>
                    </Show>
                    <button
                      type="button"
                      class="icon-btn plain"
                      aria-label={`Remove ${job.name} from the list`}
                      onClick={() => removeJob(job.id)}
                    >
                      <Icon name="close" size={18} />
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>
    </div>
  );
};
