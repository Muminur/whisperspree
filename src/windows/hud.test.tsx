// T2.4 — HUD visual feedback tests (PRD FR-5.2 / AC-5.2).
//
// These tests are deliberately presentation-focused: the Rust event emitter is
// already contract-tested in T0.5, while this suite proves its payloads have a
// useful, accessible visual representation in the HUD.

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

import { Hud, HudEventWindow, HudWindow, middleEllipsize } from "./hud";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("HUD (FR-5.2)", () => {
  it("fr_5_2_listening_renders_a_12_bar_live_meter_partial_and_active_badges", () => {
    render(
      <Hud
        view={{
          kind: "listening",
          level: 0.72,
          partial: "dictating a detailed update for the product team",
          engine: "cloud",
          language: "de",
          translation: "en",
        }}
      />,
    );

    expect(screen.getByRole("status", { name: "Listening" })).toBeInTheDocument();
    expect(screen.getAllByTestId("hud-level-bar")).toHaveLength(12);
    expect(screen.getByTestId("hud-partial-ticker")).toHaveTextContent(
      "dictating a detailed update for the product team",
    );
    expect(screen.getByText("☁")).toBeInTheDocument();
    expect(screen.getByText("DE ▸")).toBeInTheDocument();
    expect(screen.getByText("→EN")).toBeInTheDocument();
  });

  it("fr_5_2_exposes_processing_and_injecting_feedback", () => {
    const { rerender } = render(<Hud view={{ kind: "processing" }} />);
    expect(screen.getByRole("status", { name: "Processing" })).toHaveTextContent("polishing…");
    expect(screen.getByTestId("hud-spinner")).toBeInTheDocument();

    rerender(<Hud view={{ kind: "injecting" }} />);
    expect(screen.getByRole("status", { name: "Injecting" })).toHaveTextContent("✓");
  });

  it("fr_5_2_exposes_error_and_notice_states_including_fallback_badge", () => {
    const { rerender } = render(<Hud view={{ kind: "error", code: "MIC-DEV" }} />);
    expect(screen.getByRole("alert")).toHaveTextContent("MIC-DEV");

    rerender(<Hud view={{ kind: "notice", message: "Used offline cleanup", fallback: true }} />);
    expect(screen.getByRole("status", { name: "Notice" })).toHaveTextContent("Used offline cleanup");
    expect(screen.getByText("raw+")).toBeInTheDocument();
  });

  it("fr_5_2_middle_ellipsizes_a_partial_ticker_to_the_last_sixty_characters", () => {
    const text = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
    const rendered = middleEllipsize(text);

    expect(rendered).toContain("…");
    expect(rendered.length).toBeLessThanOrEqual(60);
    expect(rendered.startsWith("one")).toBe(true);
    expect(rendered.endsWith("fourteen")).toBe(true);
  });

  it("fr_5_2_post_injection_quick_menu_appears_on_hover_and_exposes_its_shell_actions", () => {
    render(<Hud view={{ kind: "injecting" }} />);
    fireEvent.mouseEnter(screen.getByTestId("hud-pill"));

    expect(screen.getByRole("menuitem", { name: "Copy raw" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Transform" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Open in History" })).toBeInTheDocument();
  });

  it("fr_5_2_hud_window_auto_hides_1_5_seconds_after_idle_and_notices_after_four_seconds", () => {
    vi.useFakeTimers();
    const { rerender } = render(<HudWindow view={{ kind: "listening", level: 0, partial: "" }} />);
    expect(screen.getByTestId("hud-window")).toBeVisible();

    rerender(<HudWindow view={{ kind: "idle" }} />);
    act(() => vi.advanceTimersByTime(1499));
    expect(screen.getByTestId("hud-window")).toBeVisible();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.getByTestId("hud-window")).not.toBeVisible();

    rerender(<HudWindow view={{ kind: "idle" }} quickMenuAvailable />);
    fireEvent.mouseEnter(screen.getByTestId("hud-pill"));
    expect(screen.getByRole("menuitem", { name: "Copy raw" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1500));
    expect(screen.getByTestId("hud-window")).toBeVisible();
    act(() => vi.advanceTimersByTime(2500));
    expect(screen.getByTestId("hud-window")).not.toBeVisible();

    rerender(<HudWindow view={{ kind: "notice", message: "Secure field" }} />);
    expect(screen.getByTestId("hud-window")).toBeVisible();
    act(() => vi.advanceTimersByTime(4000));
    expect(screen.getByTestId("hud-window")).not.toBeVisible();
  });

  it("fr_1_1_silence_only_session_state_surfaces_the_required_notice", async () => {
    const listeners = new Map<string, (payload: unknown) => void>();
    const eventListener = async <T,>(name: string, handler: (event: { payload: T }) => void) => {
      listeners.set(name, (payload) => handler({ payload: payload as T }));
      return () => listeners.delete(name);
    };
    render(<HudEventWindow eventListener={eventListener as never} />);
    await act(async () => {
      listeners.get("session:state")?.({
        state: "cancelled",
        sessionId: "session-1",
        notice: "Didn't catch anything",
      });
    });
    expect(screen.getByRole("status", { name: "Notice" })).toHaveTextContent("Didn't catch anything");
    await act(async () => {
      listeners.get("session:state")?.({ state: "post_processing", sessionId: "session-1" });
      listeners.get("session:state")?.({ state: "idle", sessionId: "session-1" });
    });
    expect(screen.getByRole("status", { name: "Notice" })).toHaveTextContent("Didn't catch anything");
    await act(async () => {
      listeners.get("session:state")?.({ state: "listening", sessionId: "session-2" });
    });
    expect(screen.getByRole("status", { name: "Listening" })).toBeInTheDocument();
  });

  it("fr_1_1_error_lifecycle_survives_idle_until_the_four_second_timeout", async () => {
    vi.useFakeTimers();
    const listeners = new Map<string, (payload: unknown) => void>();
    const eventListener = async <T,>(name: string, handler: (event: { payload: T }) => void) => {
      listeners.set(name, (payload) => handler({ payload: payload as T }));
      return () => listeners.delete(name);
    };
    render(<HudEventWindow eventListener={eventListener as never} />);
    await act(async () => {
      listeners.get("app:error")?.({ code: "MIC-DEV" });
      listeners.get("session:state")?.({ state: "idle", sessionId: "session-1" });
    });
    expect(screen.getByRole("alert")).toHaveTextContent("MIC-DEV");
    act(() => vi.advanceTimersByTime(3000));
    await act(async () => {
      listeners.get("app:error")?.({ code: "ASR-LOAD" });
    });
    expect(screen.getByRole("alert")).toHaveTextContent("ASR-LOAD");
    act(() => vi.advanceTimersByTime(3999));
    expect(screen.getByRole("alert")).toHaveTextContent("ASR-LOAD");
    act(() => vi.advanceTimersByTime(1));
    expect(screen.getByRole("status", { name: "Idle" })).toBeInTheDocument();
  });

  it("fr_1_1_recoverable_asr_error_returns_to_listening_after_timeout", async () => {
    vi.useFakeTimers();
    const listeners = new Map<string, (payload: unknown) => void>();
    const eventListener = async <T,>(name: string, handler: (event: { payload: T }) => void) => {
      listeners.set(name, (payload) => handler({ payload: payload as T }));
      return () => listeners.delete(name);
    };
    render(<HudEventWindow eventListener={eventListener as never} />);
    await act(async () => {
      listeners.get("session:state")?.({ state: "listening", sessionId: "session-1", engine: "local" });
      listeners.get("app:error")?.({ code: "ASR-SLOW", recoverable: true });
    });
    expect(screen.getByRole("alert")).toHaveTextContent("ASR-SLOW");
    act(() => vi.advanceTimersByTime(4000));
    expect(screen.getByRole("status", { name: "Listening" })).toBeInTheDocument();
    await act(async () => {
      listeners.get("audio:level")?.({ rms: 0.7, peak: 0.8 });
      listeners.get("transcript:partial")?.({ text: "still speaking" });
    });
    expect(screen.getByRole("status", { name: "Listening" })).toHaveTextContent("still speaking");
  });

  it("fr_1_1_recoverable_error_does_not_resurrect_a_finished_session", async () => {
    vi.useFakeTimers();
    const listeners = new Map<string, (payload: unknown) => void>();
    const eventListener = async <T,>(name: string, handler: (event: { payload: T }) => void) => {
      listeners.set(name, (payload) => handler({ payload: payload as T }));
      return () => listeners.delete(name);
    };
    render(<HudEventWindow eventListener={eventListener as never} />);
    await act(async () => {
      listeners.get("session:state")?.({ state: "listening", sessionId: "session-1" });
      listeners.get("app:error")?.({ code: "ASR-SLOW", recoverable: true });
      listeners.get("session:state")?.({ state: "post_processing", sessionId: "session-1" });
      listeners.get("session:state")?.({ state: "idle", sessionId: "session-1" });
    });
    act(() => vi.advanceTimersByTime(4000));
    expect(screen.getByRole("status", { name: "Idle", hidden: true })).toBeInTheDocument();
  });
});
