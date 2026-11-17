import type { Component } from "solid-js";
import { Icon } from "./Icon";

/**
 * Asked the first time "Replace the original" is chosen, in Settings or the
 * editor: what happens to originals, before anything is replaced.
 */
export const ReplaceConfirm: Component<{ onConfirm: () => void; onCancel: () => void }> = (
  props,
) => (
  <div class="confirm-box" role="alertdialog" aria-labelledby="replace-confirm-title">
    <p class="confirm-title" id="replace-confirm-title">
      <Icon name="alert" size={16} /> Replace originals?
    </p>
    <p class="confirm-text">
      Each converted file takes the place of its original, and the original goes to the Trash once
      the new file is safely written. If a conversion fails or is cancelled, the original stays as
      it was. Animations made from several pictures are always saved separately.
    </p>
    <div class="confirm-actions">
      <button type="button" class="btn btn-outline" onClick={() => props.onCancel()}>
        Cancel
      </button>
      <button type="button" class="btn btn-primary" onClick={() => props.onConfirm()}>
        Replace originals
      </button>
    </div>
  </div>
);
