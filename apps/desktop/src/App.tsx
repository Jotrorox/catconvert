import { createSignal, onCleanup, onMount } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { DropZone } from "./components/DropZone";
import { FileList } from "./components/FileList";
import { onFileDrop, onJobProgress, probeFile, startJob, type UnlistenFn } from "./lib/api";
import { DEFAULT_TARGET, type FileEntry } from "./lib/files";
import "./styles.css";

export default function App() {
  const [files, setFiles] = createStore<FileEntry[]>([]);
  const [hovering, setHovering] = createSignal(false);

  const update = (path: string, fn: (entry: FileEntry) => void) =>
    setFiles(
      (entry) => entry.path === path,
      produce((entry) => fn(entry)),
    );

  const addPaths = (paths: string[]) => {
    for (const path of paths) {
      if (files.some((entry) => entry.path === path)) continue;
      setFiles(files.length, { path, status: "probing" });
      probeFile(path)
        .then((info) =>
          update(path, (entry) => {
            entry.status = "ready";
            entry.info = info;
          }),
        )
        .catch((error: unknown) =>
          update(path, (entry) => {
            entry.status = "error";
            entry.error = String(error);
          }),
        );
    }
  };

  const convertable = () =>
    files.filter(
      (entry) =>
        entry.info &&
        DEFAULT_TARGET[entry.info.kind] !== null &&
        (!entry.job || entry.job.stage === "done"),
    );

  const convertAll = async () => {
    for (const entry of convertable()) {
      const info = entry.info;
      const target = info ? DEFAULT_TARGET[info.kind] : null;
      if (!target) continue;
      const id = await startJob(entry.path, target);
      update(entry.path, (e) => {
        e.job = { id, stage: "queued", progress: 0 };
      });
    }
  };

  onMount(() => {
    const unlisten: Promise<UnlistenFn>[] = [
      onFileDrop({ onHover: setHovering, onDrop: addPaths }),
      onJobProgress((event) =>
        setFiles(
          (entry) => entry.job?.id === event.jobId,
          "job",
          (job) => job && { ...job, stage: event.stage, progress: event.progress },
        ),
      ),
    ];
    onCleanup(() => {
      for (const p of unlisten) void p.then((fn) => fn());
    });
  });

  return (
    <main class="app">
      <header class="app-header">
        <h1>CatConvert</h1>
        <div class="actions">
          <button type="button" disabled={files.length === 0} onClick={() => setFiles([])}>
            Clear
          </button>
          <button
            type="button"
            class="primary"
            disabled={convertable().length === 0}
            onClick={() => void convertAll()}
          >
            Convert
          </button>
        </div>
      </header>
      <DropZone hovering={hovering()} />
      <FileList
        files={files}
        onRemove={(path) => setFiles((list) => list.filter((entry) => entry.path !== path))}
      />
    </main>
  );
}
