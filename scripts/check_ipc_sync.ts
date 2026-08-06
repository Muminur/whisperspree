// @ts-nocheck
import fs from "node:fs/promises";

export interface IpcSyncResult {
  rustCommands: string[];
  rustEvents: string[];
  tsCommands: string[];
  tsEvents: string[];
}

function parseQuotedList(block: string): string[] {
  return Array.from(block.matchAll(/"([^"]+)"/g), (m) => m[1]);
}

function extractRustArray(source: string, symbol: string): string[] {
  const expr = new RegExp(
    String.raw`pub\s+const\s+${symbol}\s*:\s*[^\n=]*=\s*\[([\s\S]*?)\]\s*;`,
  );
  const match = expr.exec(source);
  if (!match) {
    throw new Error(`Rust const ${symbol} not found`);
  }
  return parseQuotedList(match[1]);
}

function extractTsArray(source: string, symbol: string): string[] {
  const expr = new RegExp(
    String.raw`export\s+const\s+${symbol}\s*=\s*\[([\s\S]*?)\]\s*(?:as\s+const)?\s*;`,
  );
  const match = expr.exec(source);
  if (!match) {
    throw new Error(`TS const ${symbol} not found`);
  }
  return parseQuotedList(match[1]);
}

function compareArrays(left: string[], right: string[], label: string): void {
  const l = [...left].sort();
  const r = [...right].sort();
  if (l.length !== r.length) {
    throw new Error(
      `${label} mismatch: different lengths (rust=${l.length}, ts=${r.length})\n` +
        `rust=${JSON.stringify(l)}\n` +
        `ts=${JSON.stringify(r)}`,
    );
  }

  for (let i = 0; i < l.length; i += 1) {
    if (l[i] !== r[i]) {
      throw new Error(
        `${label} mismatch at index ${i}: rust=${l[i]} ts=${r[i]}\n` +
          `rust=${JSON.stringify(l)}\n` +
          `ts=${JSON.stringify(r)}`,
      );
    }
  }
}

export async function verifyIpcSync(): Promise<IpcSyncResult> {
  const root = process.cwd();
  const rustCommands = extractRustArray(
    await fs.readFile(`${root}/src-tauri/src/ipc/commands.rs`, "utf8"),
    "IPC_COMMANDS",
  );
  const rustEvents = extractRustArray(
    await fs.readFile(`${root}/src-tauri/src/ipc/events.rs`, "utf8"),
    "IPC_EVENTS",
  );

  const tsSource = await fs.readFile(`${root}/src/lib/ipc.ts`, "utf8");
  const tsCommands = extractTsArray(tsSource, "IPC_COMMANDS");
  const tsEvents = extractTsArray(tsSource, "IPC_EVENTS");

  compareArrays(rustCommands, tsCommands, "IPC commands");
  compareArrays(rustEvents, tsEvents, "IPC events");

  return { rustCommands, rustEvents, tsCommands, tsEvents };
}

if (process.argv[1] && process.argv[1].endsWith("/check_ipc_sync.ts")) {
  verifyIpcSync()
    .then((result) => {
      console.log(
        `ipc-sync OK (commands=${result.rustCommands.length}, events=${result.rustEvents.length})`,
      );
    })
    .catch((err) => {
      console.error(`ipc-sync FAILED: ${(err as Error).message}`);
      process.exitCode = 1;
    });
}
