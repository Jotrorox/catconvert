import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import App from "./App";

describe("App mockup", () => {
  it("starts on the Convert screen", () => {
    render(() => <App />);
    expect(screen.getByRole("heading", { name: "Drop files on the kitty" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Convert" })).toHaveAttribute("aria-current", "page");
  });

  it("opens file settings from a recipe with its format selected", () => {
    render(() => <App />);
    fireEvent.click(screen.getByRole("button", { name: /Make a GIF/ }));

    expect(screen.getByRole("heading", { name: "garden_zoomies.mov" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "GIF" })).toHaveAttribute("aria-pressed", "true");

    const audio = screen.getByRole("switch", { name: "Keep audio" });
    fireEvent.click(audio);
    expect(audio).toHaveAttribute("aria-checked", "false");

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByRole("heading", { name: "Drop files on the kitty" })).toBeInTheDocument();
  });

  it("shows the queue with the active job's progress", () => {
    render(() => <App />);
    fireEvent.click(screen.getByRole("button", { name: /Queue/ }));

    expect(screen.getByRole("heading", { name: "Queue" })).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "64");
    expect(screen.getByText("birthday_cake_pounce.mkv")).toBeInTheDocument();
  });
});
