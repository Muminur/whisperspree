// E2E-2 — deterministic Tauri IPC adapter for Playwright (CLAUDE.md §5).
//
// Installs a `window.__TAURI_INTERNALS__` shim BEFORE any @tauri-apps/api
// import touches the runtime, so `vite dev` pages behave like real windows:
// `invoke` is answered by registered handlers and §9.2 events can be emitted
// from specs through `window.__TAURI_MOCK__.emit`. Opt-in per page via the
// `?e2e=1` query parameter; production windows never install it.

export interface IpcMock {
  emit: (event: string, payload: unknown) => void;
  invoked: Array<{ cmd: string; args: unknown }>;
}

export function installIpcMock(): IpcMock {
  const w = window as unknown as {
    __TAURI_INTERNALS__?: unknown;
    __TAURI_MOCK__?: IpcMock;
  };
  if (w.__TAURI_INTERNALS__ && w.__TAURI_MOCK__) return w.__TAURI_MOCK__;

  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const invoked: Array<{ cmd: string; args: unknown }> = [];
  let snapshot = {
    microphone: "undetermined",
    accessibility: "undetermined",
    inputMonitoring: "undetermined",
  };

  const label =
    new URLSearchParams(window.location.search).get("window") ?? "main";

  w.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label },
      currentWebview: { label },
    },
    transformCallback: (callback: (result: unknown) => void) => {
      const id = window.crypto.randomUUID();
      (window as unknown as Record<string, unknown>)[`_callback_${id}`] =
        callback;
      return id;
    },
    invoke: async (cmd: string, args?: { event?: string; payload?: unknown }) => {
      if (cmd === "plugin:event|listen") {
        const event = args?.event ?? "";
        if (!listeners.has(event)) listeners.set(event, new Set());
        const callbackId = (args as { handler?: number })?.handler;
        // @tauri-apps/api v2 passes the transformed callback id; resolve it.
        const cb = (
          window as unknown as Record<string, ((e: unknown) => void) | undefined>
        )[`_callback_${callbackId}`];
        if (cb) {
          listeners.get(event)!.add((payload: unknown) =>
            cb({ event, id: 0, payload }),
          );
        }
        return null;
      }
      if (cmd === "plugin:event|unlisten") return null;
      invoked.push({ cmd, args });
      switch (cmd) {
        case "check_permissions":
          return snapshot;
        case "open_permission_pane":
          snapshot = { ...snapshot };
          return null;
        default:
          throw new Error(`ipc.mock: no handler for ${cmd}`);
      }
    },
  };

  w.__TAURI_MOCK__ = {
    invoked,
    emit: (event: string, payload: unknown) => {
      for (const listener of listeners.get(event) ?? []) listener(payload);
    },
  };
  return w.__TAURI_MOCK__;
}
