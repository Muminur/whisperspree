import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Windows } from "./windows";
import "./styles.css";

/**
 * Resolve the current Tauri window label (e.g. "hud", "settings", "main").
 *
 * `getCurrentWindow()` reads the label from the Tauri runtime internals. In a
 * plain browser (e.g. `vite dev` opened directly, or Playwright against the
 * mock harness) those internals are absent and the call throws — fall back to
 * the `main` shell instead of crashing the whole page.
 */
function resolveWindowLabel(): string {
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
