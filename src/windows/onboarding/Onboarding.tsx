import { useCallback, useEffect, useState } from "react";
import { checkPermissions, openPermissionPane, type PermissionSnapshot } from "../../lib/ipc";
import { PermissionStep, type PermissionKey } from "./steps/PermissionStep";

const steps: Array<{ key: PermissionKey; label: string; description: string }> = [
  { key: "microphone", label: "Microphone", description: "Capture speech only while dictation is active." },
  { key: "inputMonitoring", label: "Input Monitoring", description: "Listen for the global push-to-talk shortcut." },
  { key: "accessibility", label: "Accessibility", description: "Type into the focused application when permitted." },
];

export function OnboardingWindow({
  check = checkPermissions,
  open = openPermissionPane,
}: {
  check?: () => Promise<PermissionSnapshot>;
  open?: (kind: string) => Promise<void>;
}) {
  const [permissions, setPermissions] = useState<PermissionSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setPermissions(await check());
      setError(null);
    } catch {
      setError("Permissions could not be checked. Try again.");
    }
  }, [check]);

  useEffect(() => { void refresh(); }, [refresh]);

  return (
    <main data-testid="onboarding-content" className="mx-auto max-w-xl space-y-5 p-6">
      <header>
        <p className="text-xs uppercase tracking-widest text-slate-500">WhisperSpree</p>
        <h1 className="text-2xl font-semibold">Set up permissions</h1>
        <p className="mt-2 text-sm text-slate-600">Grant only the capabilities you want to use. You can change them later in System Settings.</p>
      </header>
      <section aria-label="Required permissions" className="space-y-3">
        {steps.map(({ key, label, description }) => {
          const state = permissions?.[key] ?? "undetermined";
          return (
            <PermissionStep
              key={key}
              permission={{ key, label, description }}
              state={state}
              onOpen={() => void open(key).catch(() => setError(`Could not open ${label} settings.`))}
            />
          );
        })}
      </section>
      {error && <p role="alert">{error}</p>}
      <button type="button" onClick={() => void refresh()}>Recheck permissions</button>
    </main>
  );
}
