import { type Component, For, Show } from "solid-js";
import { NAME_PLACEHOLDERS, type OutputFormat, type OutputPreview } from "../lib/api";
import { baseName } from "../lib/format";
import { dirName, extensionNote, isUsual, type OutputChoice } from "../lib/output";
import { chooseOutputLocation, resetOutput, updateOutput } from "../lib/store";
import { Icon } from "./Icon";

/**
 * Where the result goes, at the top of the editor's outcome box: the file
 * name to edit in place (a pattern when several files share the settings),
 * the folder with "Change…" for the system's save dialog, and what the
 * backend says the path will be.
 */
export const OutputField: Component<{
  choice: OutputChoice;
  /** The path the backend would save at now; `null` while it's asked or when the choice is invalid. */
  preview: OutputPreview | null;
  /** What's wrong with the chosen name or folder. */
  error: string | null;
  format: OutputFormat;
  /** Format name for the save dialog's filter, e.g. "MP4 video". */
  formatLabel: string;
  /** The name used when none is typed, without extension. */
  usualName: string;
  /** "next to the original", "next to the first picture". */
  usualPlace: string;
  /** How many files share the name, which then is a pattern. */
  files: number;
}> = (props) => {
  let field: HTMLInputElement | undefined;
  const batch = () => props.files > 1;
  const typed = () => props.choice.name ?? "";
  const note = () => extensionNote(typed(), props.format);
  const savedAs = () => (props.preview ? baseName(props.preview.path) : null);
  const folder = () => props.choice.folder ?? (props.preview ? dirName(props.preview.path) : null);
  const replacing = () => props.preview?.exists === true && props.choice.collision === "overwrite";

  const where = () => {
    if (props.choice.folder === null) return props.usualPlace;
    return `in ${baseName(props.choice.folder)}`;
  };
  const status = () => {
    const name = savedAs();
    if (!name) return `Saved ${where()}.`;
    if (batch()) return `Saved ${where()}; this one as ${name}.`;
    if (replacing()) return `Replaces ${name} ${where()}.`;
    if (props.preview?.exists) {
      return `Saved as ${name} ${where()}. The name is taken: you'll be asked whether to replace it.`;
    }
    return `Saved as ${name} ${where()}.`;
  };

  const insert = (token: string) => {
    const el = field;
    const text = typed();
    const start = el?.selectionStart ?? text.length;
    const end = el?.selectionEnd ?? text.length;
    updateOutput({ name: text.slice(0, start) + token + text.slice(end) });
    queueMicrotask(() => {
      el?.focus();
      el?.setSelectionRange(start + token.length, start + token.length);
    });
  };

  return (
    <div class="output-field">
      <div class="outcome-name">
        <Icon name="arrow-right" size={18} />
        <span class="output-name" classList={{ invalid: props.error !== null }}>
          <input
            ref={field}
            class="output-name-input"
            aria-label={batch() ? "File name pattern" : "File name"}
            aria-invalid={props.error !== null}
            aria-describedby="output-status"
            aria-errormessage={props.error ? "output-error" : undefined}
            value={typed()}
            placeholder={batch() ? "{name}" : props.usualName}
            title={
              batch()
                ? "One name for every file, filled in for each: {name} is the original's name"
                : "The new file's name. The format decides the extension."
            }
            spellcheck={false}
            autocomplete="off"
            autocapitalize="off"
            onInput={(e) => updateOutput({ name: e.currentTarget.value || null })}
          />
          <span class="output-ext">.{props.format}</span>
        </span>
      </div>

      <Show when={batch()}>
        <fieldset class="output-placeholders" aria-label="Add to the name">
          <For each={NAME_PLACEHOLDERS}>
            {(placeholder) => (
              <button
                type="button"
                class="output-token mono"
                title={`Add ${placeholder.meaning}`}
                onClick={() => insert(placeholder.token)}
              >
                {placeholder.token}
              </button>
            )}
          </For>
        </fieldset>
      </Show>

      <div class="output-where">
        <Icon name="folder" size={15} />
        <span id="output-status" class="item-meta output-status" title={folder() ?? undefined}>
          {status()}
        </span>
        <span class="output-actions">
          <button
            type="button"
            class="link-btn"
            title={batch() ? "Pick the folder to save every file in" : "Pick the folder and name"}
            onClick={() => void chooseOutputLocation(props.preview, props.formatLabel)}
          >
            Change…
          </button>
          <Show when={!isUsual(props.choice)}>
            <button type="button" class="link-btn" onClick={() => resetOutput()}>
              Reset
            </button>
          </Show>
        </span>
      </div>

      <Show when={props.error}>
        {(message) => (
          <p id="output-error" class="output-error" role="alert">
            <Icon name="alert" size={14} />
            {message()}
          </p>
        )}
      </Show>
      <Show when={!props.error && note()}>
        {(message) => <p class="item-meta output-note">{message()}</p>}
      </Show>
    </div>
  );
};
