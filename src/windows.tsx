// T0.1 window-router shells (FR-5.2 / §11).
//
// Each Tauri window loads the same `index.html`; `main.tsx` resolves the real
// window label and hands it to this router, which renders the matching shell.
// These are minimal placeholders — the real HUD, Settings, History and
// Onboarding UIs arrive in M2+. `label` is a *prop* (not read from the Tauri
// runtime here) so this component stays Tauri-free and unit-testable; never
// call `getCurrentWindow()` inside `Windows`.

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
          className="flex h-full w-full items-center justify-center bg-transparent text-sm"
        >
          HUD
        </div>
      );
    case "settings":
      return (
        <div data-testid="window-settings" className="h-full w-full p-4">
          Settings
        </div>
      );
    case "history":
      return (
        <div data-testid="window-history" className="h-full w-full p-4">
          History
        </div>
      );
    case "onboarding":
      return (
        <div data-testid="window-onboarding" className="h-full w-full p-4">
          Onboarding
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
