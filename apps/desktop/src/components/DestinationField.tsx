import { type Component, createSignal, Show } from "solid-js";
import type { Destination } from "../lib/api";
import { folderName, type OutputChoice } from "../lib/formats";
import { confirmReplace, state } from "../lib/store";
import { Icon } from "./Icon";
import { ReplaceConfirm } from "./ReplaceConfirm";

/**
 * Where one conversion's result goes, under the editor's outcome: next to
 * the original, the output folder from Settings, or (for single files) in
 * place of the original, with the same first-time confirmation as Settings.
 */
export const DestinationField: Component<{
  destination: Destination | undefined;
  onChoose: (choice: OutputChoice) => void;
  /** Animations made from several pictures can't replace them. */
  allowReplace: boolean;
  /** Says "the first picture" rather than "the original". */
  pictures?: boolean;
}> = (props) => {
  const [confirming, setConfirming] = createSignal(false);
  const current = (): OutputChoice => props.destination?.kind ?? "nextToInput";
  const folder = () => state.settings.outputFolder;
  const inFolder = () => (props.destination?.kind === "folder" ? props.destination : null);
  const choose = (choice: OutputChoice, select?: HTMLSelectElement) => {
    setConfirming(false);
    if (choice === "replaceInput" && !state.settings.replaceConfirmed) {
      // Stay on the current choice until confirmed.
      if (select) select.value = current();
      setConfirming(true);
      return;
    }
    props.onChoose(choice);
  };

  return (
    <div class="destination">
      <label class="destination-row">
        <span class="destination-label">Save</span>
        <select
          class="select select-sm"
          onChange={(e) => choose(e.currentTarget.value as OutputChoice, e.currentTarget)}
        >
          <option value="nextToInput" selected={current() === "nextToInput"}>
            {props.pictures ? "Next to the first picture" : "Next to the original"}
          </option>
          <option value="folder" selected={current() === "folder"}>
            {folder() ? `In ${folderName(folder() ?? "")}` : "In a folder…"}
          </option>
          <Show when={props.allowReplace}>
            <option value="replaceInput" selected={current() === "replaceInput"}>
              In place of the original
            </option>
          </Show>
        </select>
      </label>
      <Show when={confirming()}>
        <ReplaceConfirm
          onConfirm={() => {
            confirmReplace();
            choose("replaceInput");
          }}
          onCancel={() => setConfirming(false)}
        />
      </Show>
      <Show when={current() === "replaceInput"}>
        <p class="replace-badge">
          <Icon name="alert" size={14} /> Replaces the original; it goes to the Trash when done
        </p>
      </Show>
      <Show when={inFolder()}>
        {(dest) => (
          <p class="item-meta mono path" title={dest().path}>
            {dest().path}
            {dest().byKind ? " · sorted by kind" : ""}
          </p>
        )}
      </Show>
    </div>
  );
};
