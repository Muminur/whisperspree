// T3.2 — Settings window: dual-engine mode selection (FR-1.1).
//
// The remaining tabs arrive with their owning tasks (T4/T5/T6); this slice
// makes the §8.3 `mode` key reachable so auto/local/cloud is user-selectable.

import { useEffect, useState } from "react";
import { getSettings, updateSettings, type Settings } from "../../lib/ipc";

const MODES = ["auto", "local", "cloud"] as const;

export function Settings({ get = getSettings, save = updateSettings }: {
  get?: () => Promise<Settings>;
  save?: (patch: Partial<Settings>) => Promise<Settings>;
}) {
  const [mode, setMode] = useState<string>("auto");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    get()
      .then((s) => {
        if (active) setMode(s.mode);
      })
      .catch(() => {
        if (active) setError("Settings could not be loaded.");
      });
    return () => {
      active = false;
    };
  }, [get]);

  const changeMode = (next: string) => {
    const previous = mode;
    setMode(next);
    save({ mode: next }).catch(() => {
      setMode(previous);
      setError("Mode change could not be saved.");
    });
  };

  return (
    <div data-testid="window-settings" className="h-full w-full p-4">
      <h1 className="mb-4 text-lg font-semibold">Settings</h1>
      {error && (
        // Keep messages non-sensitive (PRD §12 P-3).
        <p role="alert" data-testid="settings-error" className="mb-3 text-sm text-red-400">
          {error}
        </p>
      )}
      <label className="flex items-center gap-3 text-sm" htmlFor="mode-select">
        Engine mode
        <select
          id="mode-select"
          data-testid="mode-select"
          value={mode}
          onChange={(e) => changeMode(e.target.value)}
        >
          {MODES.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}
