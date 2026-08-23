import { describe, it } from "vitest";
import { verifyIpcSync } from "../scripts/check_ipc_sync";

describe("IPC contract sync", () => {
  it("ipc-sync — §9.1 commands and §9.2 events stay mirrored", async () => {
    await verifyIpcSync();
  });
});
