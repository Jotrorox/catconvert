import { type Component, For, Show } from "solid-js";
import { CatPeek } from "../components/Cat";
import { Icon } from "../components/Icon";
import { QUEUE } from "../lib/mock";

export const QueueScreen: Component<{ compact: boolean }> = (props) => {
  const job = QUEUE.active;

  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1 class="display page-title">Queue</h1>
          <p class="subtitle">{QUEUE.summary}</p>
        </div>
        <Show
          when={props.compact}
          fallback={
            <div class="actions">
              <button type="button" class="btn btn-outline">
                Clear finished
              </button>
              <button type="button" class="btn btn-outline">
                <Icon name="pause" size={16} />
                Pause all
              </button>
            </div>
          }
        >
          <button type="button" class="icon-btn round" aria-label="Pause all">
            <Icon name="pause" size={20} />
          </button>
        </Show>
      </header>

      <section class="hero-job" aria-label="Converting now">
        <div class="hero-cat">
          <CatPeek sleepy />
          <Show when={!props.compact}>
            <span class="zzz" aria-hidden="true">
              z<sup>z</sup>
            </span>
          </Show>
        </div>
        <div class="hero-body">
          <div class="hero-top">
            <div>
              <Show when={!props.compact}>
                <p class="kicker">Kneading</p>
              </Show>
              <h2 class="hero-name">{job.name}</h2>
            </div>
            <p class="display hero-pct">{job.percent}%</p>
          </div>
          <div
            class="progress"
            role="progressbar"
            aria-label={`Converting ${job.name}`}
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={job.percent}
          >
            <div class="progress-fill" style={{ width: `${job.percent}%` }}>
              <Show when={!props.compact}>
                <span class="progress-knob">
                  <Icon name="paw" size={14} />
                </span>
              </Show>
            </div>
          </div>
          <p class="hero-stats">
            <span>
              <b>{job.speed}</b> speed
            </span>
            <Show when={!props.compact}>
              <span>
                <b>{job.frames}</b> / {job.totalFrames} frames
              </span>
            </Show>
            <span>
              <b>{job.left}</b> left
            </span>
            <span>
              <b>{job.written}</b>
              {props.compact ? "" : " so far"}
            </span>
          </p>
        </div>
      </section>

      <section>
        <h2 class="section-title">Up next</h2>
        <ul class="job-list">
          <For each={QUEUE.upNext}>
            {(item) => (
              <li class="job-row">
                <span class="tile" data-tone="lavender">
                  {item.position}
                </span>
                <div class="item-main">
                  <span class="item-name">{item.name}</span>
                  <span class="item-meta">
                    {props.compact ? `${item.route} · Waiting` : item.detail}
                  </span>
                </div>
                <Show when={!props.compact}>
                  <span class="pill">Waiting</span>
                  <button type="button" class="icon-btn plain" aria-label={`Remove ${item.name}`}>
                    <Icon name="close" size={18} />
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </section>

      <section>
        <h2 class="section-title">Finished</h2>
        <ul class="job-list">
          <li class="job-row">
            <span class="tile round" data-tone="green">
              <Icon name="check" size={20} />
            </span>
            <div class="item-main">
              <span class="item-name">{QUEUE.finished.name}</span>
              <span class="item-meta">
                {props.compact ? QUEUE.finished.short : QUEUE.finished.detail}
              </span>
            </div>
            <Show
              when={props.compact}
              fallback={
                <button type="button" class="btn btn-soft">
                  Show in folder
                </button>
              }
            >
              <button type="button" class="icon-btn share" aria-label="Share">
                <Icon name="share" size={22} />
              </button>
            </Show>
          </li>
          <Show when={!props.compact}>
            <li class="job-row failed">
              <span class="tile round alert">!</span>
              <div class="item-main">
                <span class="item-name">{QUEUE.failed.name}</span>
                <span class="item-meta">{QUEUE.failed.message}</span>
              </div>
              <button type="button" class="btn btn-outline">
                Retry without audio
              </button>
            </li>
          </Show>
        </ul>
      </section>
    </div>
  );
};
