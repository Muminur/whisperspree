import type { PermissionSnapshot } from "../../../lib/ipc";

export type PermissionKey = keyof PermissionSnapshot;

export function PermissionStep({
  permission,
  state,
  onOpen,
}: {
  permission: { key: PermissionKey; label: string; description: string };
  state: PermissionSnapshot[PermissionKey];
  onOpen: () => void;
}) {
  return (
    <div className="flex items-center justify-between rounded-lg border p-3" data-testid={`permission-${permission.key}`}>
      <div><h2 className="font-medium">{permission.label}</h2><p className="text-xs text-slate-500">{permission.description}</p></div>
      <div className="flex items-center gap-2">
        <span data-testid={`state-${permission.key}`} className="text-xs">{state}</span>
        {state !== "granted" && <button type="button" onClick={onOpen}>Open Settings</button>}
      </div>
    </div>
  );
}
