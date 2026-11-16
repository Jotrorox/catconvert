import { type Component, createSignal, For, Show } from "solid-js";
import { GardenScene } from "../components/Cat";
import { Icon } from "../components/Icon";
import { SAMPLE_FILE, TARGET_FORMATS, type TargetFormat } from "../lib/mock";

type Props = {
  compact: boolean;
  format: TargetFormat;
  onFormatChange: (format: TargetFormat) => void;
  onBack: () => void;
  onStart: () => void;
};

const QUALITY_STEPS = ["Tiny file", "Balanced", "Crisp"] as const;

/** File settings screen. All controls are local UI state; nothing is converted. */
export const ConvertFile: Component<Props> = (props) => {
  const [quality, setQuality] = createSignal(62);
  const [keepAudio, setKeepAudio] = createSignal(true);
  const [showCommand, setShowCommand] = createSignal(false);
  const qualityStep = () => QUALITY_STEPS[quality() < 34 ? 0 : quality() > 66 ? 2 : 1];
  const file = SAMPLE_FILE;

  const formatPicker = () => (
    <fieldset class="field">
      <legend class="field-label">Turn it into</legend>
      <div class="segmented">
        <For each={TARGET_FORMATS}>
          {(format) => (
            <button
              type="button"
              aria-pressed={props.format === format}
              onClick={() => props.onFormatChange(format)}
            >
              {format}
            </button>
          )}
        </For>
      </div>
    </fieldset>
  );

  const options = () => (
    <>
      <div class="field">
        <label class="field-label" for="quality">
          Quality
        </label>
        <input
          id="quality"
          class="range"
          type="range"
          min="0"
          max="100"
          value={quality()}
          style={{ "--pct": `${quality()}%` }}
          onInput={(event) => setQuality(event.currentTarget.valueAsNumber)}
        />
        <div class="range-scale">
          <For each={QUALITY_STEPS}>
            {(step) => <span classList={{ active: qualityStep() === step }}>{step}</span>}
          </For>
        </div>
      </div>
      <div class="two-col">
        <label class="field">
          <span class="field-label">Size</span>
          <select class="select">
            <For each={file.sizes}>
              {(size) => <option selected={size === file.defaultSize}>{size}</option>}
            </For>
          </select>
        </label>
        <label class="field">
          <span class="field-label">Frame rate</span>
          <select class="select">
            <For each={file.frameRates}>{(rate) => <option>{rate}</option>}</For>
          </select>
        </label>
      </div>
      <div class="toggle-row">
        <div>
          <span class="toggle-title" id="keep-audio">
            Keep audio
          </span>
          <span class="item-meta">{file.audio}</span>
        </div>
        <button
          type="button"
          class="switch"
          role="switch"
          aria-checked={keepAudio()}
          aria-labelledby="keep-audio"
          onClick={() => setKeepAudio(!keepAudio())}
        />
      </div>
    </>
  );

  const command = () => (
    <pre class="command-text">
      <code>{file.command}</code>
    </pre>
  );

  return (
    <Show
      when={props.compact}
      fallback={
        <div class="page">
          <header class="file-header">
            <button
              type="button"
              class="icon-btn square"
              aria-label="Back"
              onClick={() => props.onBack()}
            >
              <Icon name="chevron-left" size={22} />
            </button>
            <div class="file-heading">
              <h1 class="display file-title">{file.name}</h1>
              <p class="item-meta">{file.meta}</p>
            </div>
            <button type="button" class="btn btn-primary btn-lg" onClick={() => props.onStart()}>
              <Icon name="paw" size={18} />
              Start converting
            </button>
          </header>

          <div class="file-grid">
            <div class="stack">
              <div class="preview">
                <GardenScene class="preview-art" />
                <button type="button" class="play" aria-label="Play preview">
                  <Icon name="play" size={22} />
                </button>
              </div>
              <section class="card trim">
                <div class="row-between">
                  <h2 class="label-strong">Trim</h2>
                  <span class="item-meta">{file.trim.summary}</span>
                </div>
                <div class="trim-track">
                  <div
                    class="trim-range"
                    style={{ left: `${file.trim.fromPct}%`, right: `${100 - file.trim.toPct}%` }}
                  >
                    <span class="trim-handle start" />
                    <span class="trim-handle end" />
                  </div>
                </div>
                <div class="two-col">
                  <label class="field">
                    <span class="field-label">Start</span>
                    <input class="input mono" value={file.trim.start} />
                  </label>
                  <label class="field">
                    <span class="field-label">End</span>
                    <input class="input mono" value={file.trim.end} />
                  </label>
                </div>
              </section>
            </div>

            <section class="card settings">
              {formatPicker()}
              {options()}
              <div class="estimate">
                <Icon name="paw" size={24} />
                <p>
                  <strong>{file.estimate.size}</strong> when it's done
                  <br />
                  That's {file.estimate.saving} than the original.
                </p>
              </div>
              <details class="command">
                <summary>Show ffmpeg command</summary>
                {command()}
              </details>
            </section>
          </div>
        </div>
      }
    >
      <div class="page file-compact">
        <header class="file-header">
          <button
            type="button"
            class="icon-btn round"
            aria-label="Back"
            onClick={() => props.onBack()}
          >
            <Icon name="chevron-left" size={24} />
          </button>
          <div class="file-heading">
            <h1 class="file-title compact">{file.name}</h1>
            <p class="item-meta">{file.shortMeta}</p>
          </div>
        </header>

        <div class="preview">
          <GardenScene class="preview-art" />
          <span class="trim-pill">Trim {file.trim.short}</span>
          <button type="button" class="icon-btn scissors" aria-label="Trim">
            <Icon name="scissors" size={22} />
          </button>
        </div>

        {formatPicker()}
        <section class="card settings">{options()}</section>

        <div class="estimate inline">
          <Icon name="paw" size={24} />
          <span>
            <strong>{file.estimate.size}</strong> · {file.estimate.saving}
          </span>
          <button
            type="button"
            class="link-btn"
            aria-expanded={showCommand()}
            onClick={() => setShowCommand(!showCommand())}
          >
            Command
          </button>
        </div>
        <Show when={showCommand()}>{command()}</Show>

        <div class="sticky-cta">
          <button type="button" class="btn btn-primary btn-block" onClick={() => props.onStart()}>
            <Icon name="paw" size={20} />
            Start converting
          </button>
        </div>
      </div>
    </Show>
  );
};
