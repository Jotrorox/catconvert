import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";
import type { FileEntry } from "../lib/files";
import { FileList } from "./FileList";

const ready: FileEntry = {
  path: "/Users/cat/Pictures/whiskers.jpg",
  status: "ready",
  info: {
    path: "/Users/cat/Pictures/whiskers.jpg",
    fileName: "whiskers.jpg",
    extension: "jpg",
    sizeBytes: 2_500_000,
    kind: "image",
  },
};

describe("FileList", () => {
  it("shows a placeholder when empty", () => {
    render(() => <FileList files={[]} />);
    expect(screen.getByText("No files yet.")).toBeInTheDocument();
  });

  it("renders probed info for each file", () => {
    const failed: FileEntry = { path: "C:\\tmp\\gone.mp4", status: "error", error: "not found" };
    const probing: FileEntry = { path: "/tmp/song.flac", status: "probing" };
    render(() => <FileList files={[ready, failed, probing]} />);

    expect(screen.getAllByRole("listitem")).toHaveLength(3);
    expect(screen.getByText("whiskers.jpg")).toBeInTheDocument();
    expect(screen.getByText(".jpg")).toBeInTheDocument();
    expect(screen.getByText("2.5 MB")).toBeInTheDocument();
    expect(screen.getByText("image")).toBeInTheDocument();
    expect(screen.getByText("gone.mp4")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("not found");
    expect(screen.getByText("Probing…")).toBeInTheDocument();
  });

  it("shows job progress and supports removal", () => {
    const onRemove = vi.fn();
    const converting: FileEntry = { ...ready, job: { id: 1, stage: "running", progress: 0.42 } };
    render(() => <FileList files={[converting]} onRemove={onRemove} />);

    expect(screen.getByText("42%")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("value", "0.42");

    fireEvent.click(screen.getByRole("button", { name: `Remove ${ready.path}` }));
    expect(onRemove).toHaveBeenCalledWith(ready.path);
  });
});
