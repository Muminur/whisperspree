// T2.4 — floating HUD view (PRD FR-5.2).
//
// The native window's transparent/non-activating/always-on-top characteristics
// are intentionally owned by `tauri.conf.json`.  This file owns only the
// event-driven React view, keeping it deterministic and testable in jsdom.

import { listen, type Event } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "react";

import type {
  AppErrorPayload,
  AudioLevelPayload,
  InjectDonePayload,
  LanguageDetectedPayload,
  PostprocessDonePayload,
  SessionStatePayload,
  TranscriptPartialPayload,
} from "../../lib/ipc";
import { getSettings } from "../../lib/ipc";
import { LevelMeter } from "./LevelMeter";
import { QuickMenu } from "./QuickMenu";
import { Ticker } from "./Ticker";
// FR-5.2 click-through toggle rides Tauri's built-in window API.

const IDLE_HIDE_MS = 1_500;
const NOTICE_HIDE_MS = 4_000;

// Headless-safe wrapper: outside a real Tauri webview there is no window
// metadata, so the FR-5.2 cursor toggle degrades to a no-op.
function syncHudClickThrough(ignore: boolean) {
  try {
    void getCurrentWindow().setIgnoreCursorEvents(ignore);
  } catch {
    // No Tauri runtime (tests / plain browser).
  }
}

function syncNativeHudVisibility(visible: boolean) {
  try {
    const operation = visible ? getCurrentWindow().show() : getCurrentWindow().hide();
    void operation.catch(() => undefined);
  } catch {
    // Vite/RTL runs do not have Tauri's webview runtime.
  }
}

export type HudView =
  | { kind: "idle" }
  | {
      kind: "listening";
      level: number;
      partial: string;
      engine?: string;
      language?: string;
      translation?: string;
    }
  | { kind: "processing" }
  | { kind: "injecting" }
  | { kind: "error"; code: string }
  | { kind: "notice"; message: string; fallback?: boolean };

/** Retain the beginning and end of ASR partials without making the pill grow. */
export { middleEllipsize } from "./Ticker";

function EngineBadge({ engine }: { engine?: string }) {
  if (!engine) return null;
  const isCloud = engine === "cloud" || engine === "deepgram";
  return <span className="hud-badge" title={isCloud ? "Cloud engine" : "Local engine"}>{isCloud ? "☁" : "⌂"}</span>;
}

export function Hud({ view, quickMenuAvailable = false }: { view: HudView; quickMenuAvailable?: boolean }) {
  const [quickMenu, setQuickMenu] = useState(false);
  const canOpenQuickMenu = view.kind === "injecting" || quickMenuAvailable;

  useEffect(() => {
    if (!canOpenQuickMenu) setQuickMenu(false);
  }, [canOpenQuickMenu]);

  // FR-5.2 click-through: swallow all cursor events except while the pointer
  // is over the quick menu, using Tauri's window API directly (no §9.1 command).
  useEffect(() => {
    syncHudClickThrough(!quickMenu);
  }, [quickMenu]);

  return (
    <div
      data-testid="hud-pill"
      className="hud-pill"
      onMouseEnter={() => canOpenQuickMenu && setQuickMenu(true)}
      onMouseLeave={() => setQuickMenu(false)}
    >
      {view.kind === "idle" && <div role="status" aria-label="Idle" className="hud-status">Ready</div>}

      {view.kind === "listening" && (
        <div role="status" aria-label="Listening" className="hud-listening">
          <span className="hud-listening-dot" aria-hidden="true" />
          <LevelMeter level={view.level} />
          <Ticker text={view.partial} />
          <EngineBadge engine={view.engine} />
          {view.language && <span className="hud-badge">{view.language.toUpperCase()} ▸</span>}
          {view.translation && <span className="hud-badge">→{view.translation.toUpperCase()}</span>}
        </div>
      )}

      {view.kind === "processing" && (
        <div role="status" aria-label="Processing" className="hud-status">
          <span data-testid="hud-spinner" className="hud-spinner" aria-hidden="true" /> polishing…
        </div>
      )}

      {view.kind === "injecting" && <div role="status" aria-label="Injecting" className="hud-status">✓</div>}

      {view.kind === "error" && <div role="alert" className="hud-error">{view.code}</div>}

      {view.kind === "notice" && (
        <div role="status" aria-label="Notice" className="hud-status">
          {view.message} {view.fallback && <span className="hud-badge">raw+</span>}
        </div>
      )}

      <QuickMenu visible={quickMenu} />
    </div>
  );
}

/** Applies the FR-5.2 idle/notice timeout policy around the visual HUD. */
export function HudWindow({ view, quickMenuAvailable = false }: { view: HudView; quickMenuAvailable?: boolean }) {
  const [visible, setVisible] = useState(view.kind !== "idle");
  const isInitialView = useRef(true);

  useEffect(() => {
    if (view.kind === "idle" && isInitialView.current) {
      syncNativeHudVisibility(false);
      isInitialView.current = false;
      return;
    }
    isInitialView.current = false;

    if (view.kind !== "idle" && view.kind !== "notice") {
      setVisible(true);
      // The HUD is intentionally created hidden in tauri.conf.json. Sync that
      // native visibility with React without ever focusing the window.
      syncNativeHudVisibility(true);
      return;
    }

    setVisible(true);
    syncNativeHudVisibility(true);
    const timeout = window.setTimeout(
      () => {
        setVisible(false);
        syncNativeHudVisibility(false);
      },
      view.kind === "idle" && quickMenuAvailable ? NOTICE_HIDE_MS : view.kind === "idle" ? IDLE_HIDE_MS : NOTICE_HIDE_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [view, quickMenuAvailable]);

  return (
    <div data-testid="hud-window" className="hud-window" hidden={!visible} aria-hidden={!visible}>
      <Hud view={view} quickMenuAvailable={quickMenuAvailable} />
    </div>
  );
}

/**
 * Subscribe the Tauri HUD window to §9.2 events.  The optional listener keeps
 * the pure `HudWindow` usable in browser tests and the forthcoming mock-IPC
 * harness, while production uses Tauri's native event transport.
 */
export function HudEventWindow({ eventListener = listen }: { eventListener?: typeof listen }) {
  const [view, setView] = useState<HudView>({ kind: "idle" });
  const [level, setLevel] = useState(0);
  const [partial, setPartial] = useState("");
  const [engine, setEngine] = useState<string>();
  const [language, setLanguage] = useState<string>();
  const [translation, setTranslation] = useState<string>();
  const [quickMenuAvailable, setQuickMenuAvailable] = useState(false);
  const errorActive = useRef(false);
  const recoverableErrorActive = useRef(false);
  const noticeActive = useRef(false);
  const noticeSessionId = useRef<string>();
  const currentSessionId = useRef<string>();
  const errorSessionId = useRef<string>();
  const errorTimer = useRef<number>();
  const noticeTimer = useRef<number>();

  useEffect(() => {
    let active = true;
    void getSettings()
      .then((settings) => {
        if (active && settings.translation.enabled) setTranslation(settings.translation.targetLanguage);
      })
      // A mock/browser HUD can render without a Tauri command bridge.
      .catch(() => undefined);
    return () => { active = false; };
  }, []);

  useEffect(() => {
    let active = true;
    const unlisten: Array<() => void> = [];
    const subscribe = <Payload,>(name: string, handler: (payload: Payload) => void) => {
      void eventListener<Payload>(name, (event: Event<Payload>) => handler(event.payload))
        .then((dispose) => {
          if (active) unlisten.push(dispose);
          else dispose();
        })
        // A direct browser load has no Tauri transport. Keep the inert HUD
        // rather than turning an unavailable IPC bridge into a visible error.
        .catch(() => undefined);
    };

    subscribe<SessionStatePayload>("session:state", ({ state, engine: nextEngine, notice, sessionId }) => {
      currentSessionId.current = sessionId;
      if (
        recoverableErrorActive.current
        && errorSessionId.current === sessionId
        && state !== "listening"
      ) {
        recoverableErrorActive.current = false;
        errorActive.current = false;
        if (errorTimer.current !== undefined) window.clearTimeout(errorTimer.current);
      }
      if (nextEngine) setEngine(nextEngine);
      if (notice) {
        noticeActive.current = true;
        noticeSessionId.current = sessionId;
        setView({ kind: "notice", message: notice });
        if (noticeTimer.current !== undefined) window.clearTimeout(noticeTimer.current);
        noticeTimer.current = window.setTimeout(() => { noticeActive.current = false; }, NOTICE_HIDE_MS);
      }
      else if (state === "listening") {
        noticeActive.current = false;
        errorActive.current = false;
        recoverableErrorActive.current = false;
        noticeSessionId.current = undefined;
        errorSessionId.current = undefined;
        if (noticeTimer.current !== undefined) window.clearTimeout(noticeTimer.current);
        if (errorTimer.current !== undefined) window.clearTimeout(errorTimer.current);
        setView({ kind: "listening", level, partial, engine: nextEngine ?? engine, language, translation });
      }
      else if (noticeActive.current && noticeSessionId.current === sessionId) return;
      else if (state === "injecting") setView({ kind: "injecting" });
      else if (state === "error") setView({ kind: "error", code: "SESSION" });
      else if (state === "idle" || state === "cancelled") {
        if (!errorActive.current) setView({ kind: "idle" });
      }
      else setView({ kind: "processing" });
    });
    subscribe<AudioLevelPayload>("audio:level", ({ rms, peak }) => {
      const nextLevel = Math.max(0, Math.min(1, Math.max(rms, peak)));
      setLevel(nextLevel);
      setView((current) => current.kind === "listening" ? { ...current, level: nextLevel } : current);
    });
    subscribe<TranscriptPartialPayload>("transcript:partial", ({ text }) => {
      setPartial(text);
      setView((current) => current.kind === "listening" ? { ...current, partial: text } : current);
    });
    subscribe<LanguageDetectedPayload>("language:detected", ({ code }) => {
      setLanguage(code);
      setView((current) => current.kind === "listening" ? { ...current, language: code } : current);
    });
    subscribe<PostprocessDonePayload>("postprocess:done", ({ fallbackUsed }) => {
      if (fallbackUsed) setView({ kind: "notice", message: "Offline cleanup applied", fallback: true });
    });
    subscribe<InjectDonePayload>("inject:done", () => {
      setView({ kind: "injecting" });
      setQuickMenuAvailable(true);
      window.setTimeout(() => setQuickMenuAvailable(false), NOTICE_HIDE_MS);
    });
    subscribe<AppErrorPayload>("app:error", ({ code, recoverable }) => {
      errorActive.current = true;
      recoverableErrorActive.current = recoverable;
      errorSessionId.current = currentSessionId.current;
      if (errorTimer.current !== undefined) window.clearTimeout(errorTimer.current);
      setView({ kind: "error", code });
      errorTimer.current = window.setTimeout(() => {
        if (recoverable && currentSessionId.current !== errorSessionId.current) return;
        errorActive.current = false;
        recoverableErrorActive.current = false;
        if (recoverable) {
          setView({ kind: "listening", level, partial, engine, language, translation });
        } else {
          setView({ kind: "idle" });
        }
      }, NOTICE_HIDE_MS);
    });

    return () => {
      active = false;
      unlisten.forEach((dispose) => dispose());
    };
  // Listener identity is deliberately a dependency: test adapters may change.
  // The state values are read from setter callbacks where live updates matter.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eventListener]);

  return <HudWindow view={view} quickMenuAvailable={quickMenuAvailable} />;
}
