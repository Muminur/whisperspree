import { defineConfig } from "@playwright/test";

// E2E-2 (CLAUDE.md §5): React windows served by Vite against the mock IPC
// adapter. Real hotkeys/mic/injection stay out of scope — manual QA sheets.
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  use: {
    baseURL: "http://localhost:1420",
    // This dev machine is macOS 13 (Intel); current Playwright builds ship no
    // bundled Chromium for it, so drive the installed Google Chrome instead
    // (also preinstalled on GitHub runners).
    channel: "chrome",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "pnpm dev --strictPort",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
