import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed, known port and never clears the dev console so the
// Rust build/log output stays visible (§4.1, §11).
// https://v2.tauri.app/reference/config/#buildconfig
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: "jsdom",
    setupFiles: "./vitest.setup.ts",
    include: ["src/**/*.test.{ts,tsx}"],
    // No frontend tests exist yet; the first ones arrive later in T0.1
    // (windows.test.tsx) and at T0.5 (ipc-sync).
    passWithNoTests: true,
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      // Thresholds only bind when `--coverage` is passed; plain `pnpm test`
      // (the GATE) does not enforce them until real TS lands (T0.5).
      thresholds: {
        lines: 85,
        branches: 85,
        functions: 85,
        statements: 85,
      },
    },
  },
});
