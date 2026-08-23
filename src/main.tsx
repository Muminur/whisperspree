import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Windows } from "./windows";
import "./styles.css";

// E2E-2: `?e2e=1` pages run against the mock IPC adapter instead of a real
// Tauri runtime (CLAUDE.md §5). Must install before any window API call.
const e2eParams = new URLSearchParams(window.location.search);
if (e2eParams.has("e2e")) {
  void import("./lib/ipc.mock").then((m) => m.installIpcMock());
}

/**
 * Resolve the current Tauri window label (e.g. "hud", "settings", "main").
 *
 * Order: explicit `?window=` override (E2E harness) → Tauri runtime → `main`.
 */
function resolveWindowLabel(): string {
  const overridden = e2eParams.get("window");
  if (overridden) return overridden;
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Windows label={resolveWindowLabel()} />
  </StrictMode>,
);
