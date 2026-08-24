// T3.2 — Settings mode selection (FR-1.1 dual-engine modes).
const updateSettings = vi.hoisted(() => vi.fn().mockResolvedValue({}));
const getSettings = vi.hoisted(() =>
  vi.fn().mockResolvedValue({
    version: 1,
    mode: "auto",
    hotkey: {},
    audio: {},
    asr: { cloudProvider: "deepgram" },
    postprocess: {},
    translation: {},
    context: {},
    injection: {},
    history: {},
    appearance: {},
  }),
);
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) =>
    cmd === "update_settings" ? updateSettings(args) : Promise.resolve({}),
}));
vi.mock("../../lib/ipc", async (orig) => ({
  ...(await orig()),
  getSettings,
  updateSettings: (patch: Partial<import("../../lib/ipc").Settings>) => updateSettings(patch),
}));

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Settings } from "./Settings";

afterEach(() => {
  cleanup();
});

describe("Settings mode UI (FR-1.1 / T3.2)", () => {
  it("t3_2_renders_the_mode_selector_with_the_three_prd_modes", async () => {
    render(<Settings />);
    await waitFor(() => expect(screen.getByTestId("mode-select")).toHaveValue("auto"));
    for (const m of ["auto", "local", "cloud"]) {
      expect(screen.getByRole("option", { name: m })).toBeInTheDocument();
    }
  });

  it("t3_2_changing_the_mode_persists_via_update_settings", async () => {
    render(<Settings />);
    await waitFor(() => expect(screen.getByTestId("mode-select")).toHaveValue("auto"));
    fireEvent.change(screen.getByTestId("mode-select"), { target: { value: "cloud" } });
    await waitFor(() =>
      expect(updateSettings).toHaveBeenCalledWith(expect.objectContaining({ mode: "cloud" })),
    );
  });
});
