import { type Component, For, Match, Show, Switch } from "solid-js";
import { baseName, type FileEntry } from "../lib/files";
import { formatBytes } from "../lib/format";

type Props = {
  files: readonly FileEntry[];
  onRemove?: (path: string) => void;
};

export const FileList: Component<Props> = (props) => (
  <Show when={props.files.length > 0} fallback={<p class="empty">No files yet.</p>}>
    <ul class="file-list" aria-label="Files">
      <For each={props.files}>
        {(file) => (
          <li class="file" data-status={file.status}>
            <div class="file-main">
              <span class="file-name" title={file.path}>
                {file.info?.fileName ?? baseName(file.path)}
              </span>
              <Switch>
                <Match when={file.status === "probing"}>
                  <span class="file-meta">Probing…</span>
                </Match>
                <Match when={file.status === "error"}>
                  <span class="file-meta error" role="alert">
                    {file.error}
                  </span>
                </Match>
                <Match when={file.info}>
                  {(info) => (
                    <span class="file-meta">
                      <span class="badge" data-kind={info().kind}>
                        {info().kind}
                      </span>
                      <span>{info().extension ? `.${info().extension}` : "no extension"}</span>
                      <span>{formatBytes(info().sizeBytes)}</span>
                    </span>
                  )}
                </Match>
              </Switch>
            </div>
            <Show when={file.job}>
              {(job) => (
                <div class="job">
                  <progress
                    max="1"
                    value={job().progress}
                    aria-label={`Converting ${file.info?.fileName ?? file.path}`}
                  />
                  <span class="job-label">
                    {job().stage === "done" ? "Done" : `${Math.round(job().progress * 100)}%`}
                  </span>
                </div>
              )}
            </Show>
            <Show when={props.onRemove}>
              {(onRemove) => (
                <button
                  type="button"
                  class="icon-button"
                  aria-label={`Remove ${file.path}`}
                  onClick={() => onRemove()(file.path)}
                >
                  ×
                </button>
              )}
            </Show>
          </li>
        )}
      </For>
    </ul>
  </Show>
);
