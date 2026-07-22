// T0.1 — frontend window-router shells (RED phase, tests-first per CLAUDE.md §3).
//
// PRD refs: FR-5.2 (§6) — WhisperSpree ships four distinct window shells (HUD,
// Settings, History, Onboarding) plus a `main` fallback. §11 lists them under
// src/windows/. This suite pins the router contract BEFORE src/windows.tsx
// exists, so it must currently fail because the `./windows` import cannot
// resolve (red for the right reason — a missing module, not a bad assertion).
//
// Contract asserted here:
//   • `Windows` is a named export.
//   • It takes a single prop `label: string`.
//   • labels "hud" | "settings" | "history" | "onboarding" | "main" each render
//     a DISTINCT shell, identified by data-testid `window-<label>`.
//   • Any unknown label falls back to the `window-main` shell.
//
// DESIGN NOTE FOR THE GREEN IMPLEMENTER (do NOT create src/windows.tsx here):
//   main.tsx resolves the real window label at runtime via
//   `getCurrentWindow().label` from @tauri-apps/api/window and passes it into
//   <Windows label={...} />. The label is a *prop* precisely so these tests
//   need no Tauri mock — keep it that way (never call getCurrentWindow() inside
//   the Windows component).

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";

// This import is expected to FAIL until src/windows.tsx is implemented (T0.1 green).
import { Windows } from "./windows";

// The vitest config does not set `globals: true`, so RTL's auto-cleanup is not
// registered — do it explicitly to keep the jsdom container clean per test.
afterEach(cleanup);

/** The four real FR-5.2 window labels plus the `main` fallback shell. */
const KNOWN_LABELS = [
  "hud",
  "settings",
  "history",
  "onboarding",
  "main",
] as const;

describe("Windows router (FR-5.2)", () => {
  it("fr_5_2_hud_shell_renders_for_hud_label", () => {
    render(<Windows label="hud" />);
    expect(screen.getByTestId("window-hud")).toBeInTheDocument();
  });

  it("fr_5_2_settings_shell_renders_for_settings_label", () => {
    render(<Windows label="settings" />);
    expect(screen.getByTestId("window-settings")).toBeInTheDocument();
  });

  it("fr_5_2_history_shell_renders_for_history_label", () => {
    render(<Windows label="history" />);
    expect(screen.getByTestId("window-history")).toBeInTheDocument();
  });

  it("fr_5_2_onboarding_shell_renders_for_onboarding_label", () => {
    render(<Windows label="onboarding" />);
    expect(screen.getByTestId("window-onboarding")).toBeInTheDocument();
  });

  it("fr_5_2_main_shell_renders_for_main_label", () => {
    render(<Windows label="main" />);
    expect(screen.getByTestId("window-main")).toBeInTheDocument();
  });

  it("fr_5_2_each_label_renders_a_distinct_shell", () => {
    // Each known label must render exactly its own shell and none of the others.
    for (const label of KNOWN_LABELS) {
      const { unmount } = render(<Windows label={label} />);
      expect(screen.getByTestId(`window-${label}`)).toBeInTheDocument();
      for (const other of KNOWN_LABELS) {
        if (other !== label) {
          expect(screen.queryByTestId(`window-${other}`)).toBeNull();
        }
      }
      unmount();
    }
  });

  it("fr_5_2_unknown_label_falls_back_to_main_shell", () => {
    render(<Windows label="definitely-not-a-real-window" />);
    expect(screen.getByTestId("window-main")).toBeInTheDocument();
    // The fallback must not masquerade as a real feature window.
    expect(screen.queryByTestId("window-hud")).toBeNull();
    expect(screen.queryByTestId("window-settings")).toBeNull();
    expect(screen.queryByTestId("window-history")).toBeNull();
    expect(screen.queryByTestId("window-onboarding")).toBeNull();
  });
});
