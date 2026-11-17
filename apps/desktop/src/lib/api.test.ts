import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { dragFiles, onFileDrop, onOpenedFiles } from "./api";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: class {
    onmessage: (message: unknown) => void = () => {};
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));

type DropPayload = { type: "enter" | "over" | "leave" | "drop"; paths?: string[] };

beforeEach(() => {
  vi.clearAllMocks();
});

describe("onOpenedFiles", () => {
  it("listens before taking the backlog, then forwards new files", async () => {
    const order: string[] = [];
    let emit: (paths: string[]) => void = () => {};
    vi.mocked(listen).mockImplementation(async (_name, handler) => {
      order.push("listen");
      emit = (paths) => handler({ event: "open-files", id: 1, payload: paths } as never);
      return () => {};
    });
    vi.mocked(invoke).mockImplementation(async (name) => {
      order.push(name);
      return ["/clips/early.mov"];
    });

    const handler = vi.fn();
    await onOpenedFiles(handler);
    expect(order).toEqual(["listen", "take_opened_files"]);
    expect(handler).toHaveBeenCalledWith(["/clips/early.mov"]);

    emit(["/clips/later.mov", "/photos/cat.jpg"]);
    expect(handler).toHaveBeenLastCalledWith(["/clips/later.mov", "/photos/cat.jpg"]);
  });

  it("stays quiet when nothing was waiting", async () => {
    vi.mocked(listen).mockResolvedValue(() => {});
    vi.mocked(invoke).mockResolvedValue([]);
    const handler = vi.fn();
    await onOpenedFiles(handler);
    expect(handler).not.toHaveBeenCalled();
  });
});

describe("dragFiles", () => {
  it("ignores its own files dropped back on the window", async () => {
    let drop: (payload: DropPayload) => void = () => {};
    vi.mocked(getCurrentWebview).mockReturnValue({
      onDragDropEvent: async (handler: (event: { payload: DropPayload }) => void) => {
        drop = (payload) => handler({ payload });
        return () => {};
      },
    } as never);
    let channel: { onmessage: (message: unknown) => void } | undefined;
    vi.mocked(invoke).mockImplementation(async (_name, args) => {
      channel = (args as { onEvent: typeof channel }).onEvent;
      return null;
    });

    const onHover = vi.fn();
    const onDrop = vi.fn();
    await onFileDrop({ onHover, onDrop });

    const dragged = dragFiles(["/clips/clip.mp4"], "data:image/png;base64,AAAA");
    expect(invoke).toHaveBeenCalledWith(
      "plugin:drag|start_drag",
      expect.objectContaining({ item: ["/clips/clip.mp4"], options: { mode: "copy" } }),
    );
    drop({ type: "enter", paths: ["/clips/clip.mp4"] });
    drop({ type: "drop", paths: ["/clips/clip.mp4"] });
    expect(onHover).not.toHaveBeenCalledWith(true);
    expect(onDrop).not.toHaveBeenCalled();

    channel?.onmessage({ result: "Dropped" });
    await dragged;
    drop({ type: "drop", paths: ["/photos/cat.jpg"] });
    expect(onDrop).toHaveBeenCalledWith(["/photos/cat.jpg"]);
  });

  it("gives up waiting for the plugin at the next press", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    const dragged = dragFiles(["/clips/clip.mp4"], "");
    window.dispatchEvent(new Event("pointerdown"));
    await expect(dragged).resolves.toBeUndefined();
  });

  it("rejects when the drag can't start", async () => {
    vi.mocked(invoke).mockRejectedValue("expected an image/png base64 image string");
    await expect(dragFiles(["/clips/clip.mp4"], "")).rejects.toBe(
      "expected an image/png base64 image string",
    );
  });
});
