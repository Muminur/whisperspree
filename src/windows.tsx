// T0.1 window-router shells (FR-5.2 / §11).
//
// Each Tauri window loads the same `index.html`; `main.tsx` resolves the real
// window label and hands it to this router, which renders the matching shell.
// These are minimal placeholders — the real HUD, Settings, History and
// Onboarding UIs arrive in M2+. `label` is a *prop* (not read from the Tauri
// runtime here) so this component stays Tauri-free and unit-testable; never
// call `getCurrentWindow()` inside `Windows`.

import { HudEventWindow } from "./windows/hud";
import { History } from "./windows/history";
import { OnboardingWindow } from "./windows/onboarding";
import { Settings } from "./windows/settings";

/**
 * Route a window `label` to its shell.
 *
 * The four FR-5.2 windows (`hud`, `settings`, `history`, `onboarding`) each get
 * a distinct shell keyed by `data-testid="window-<label>"`. `main` and any
 * unknown label fall back to the `main` shell.
 */
export function Windows({ label }: { label: string }) {
  switch (label) {
    case "hud":
      return (
        <div
          data-testid="window-hud"
          className="h-full w-full bg-transparent"
        >
          <HudEventWindow />
        </div>
      );
    case "settings":
      return <Settings />;
    case "history":
      return <History />;
    case "onboarding":
      return (
        <div data-testid="window-onboarding" className="h-full w-full">
          <OnboardingWindow />
        </div>
      );
    default:
      // `main` and any unrecognised label render the neutral main shell.
      return (
        <div data-testid="window-main" className="h-full w-full p-4">
          WhisperSpree
        </div>
      );
  }
}
