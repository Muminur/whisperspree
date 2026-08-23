// T0.5 — Tauri entrypoint tests for the window-label bridge (PRD §9.1).

import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./styles.css", () => ({}));
const { createRoot, getCurrentWindow, render } = vi.hoisted(() => {
  const render = vi.fn();
  return {
    createRoot: vi.fn(() => ({ render })),
    getCurrentWindow: vi.fn(),
    render,
  };
});

vi.mock("react-dom/client", () => ({ createRoot }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow }));

type WindowsElement = ReactElement<{ label: string }>;
type StrictModeElement = ReactElement<{ children: WindowsElement }>;

function renderedWindowLabel(): string {
  const tree = render.mock.calls[0]?.[0] as StrictModeElement;
  return tree.props.children.props.label;
}

describe("main entrypoint window routing", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    createRoot.mockReturnValue({ render });
    document.body.innerHTML = '<div id="root"></div>';
  });

  afterEach(() => {
    document.body.replaceChildren();
  });

  it("fr_0_5_entrypoint_passes_the_current_tauri_window_label_to_the_router", async () => {
    getCurrentWindow.mockReturnValue({ label: "settings" });

    await import("./main");

    expect(getCurrentWindow).toHaveBeenCalledExactlyOnceWith();
    expect(createRoot).toHaveBeenCalledExactlyOnceWith(document.getElementById("root"));
    expect(render).toHaveBeenCalledTimes(1);
    expect(renderedWindowLabel()).toBe("settings");
  });

  it("ec_0_5_entrypoint_falls_back_to_main_when_tauri_runtime_is_absent", async () => {
    getCurrentWindow.mockImplementation(() => {
      throw new Error("Tauri runtime is unavailable in a browser");
    });

    await import("./main");

    expect(renderedWindowLabel()).toBe("main");
  });
});
