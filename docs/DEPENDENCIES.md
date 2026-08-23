# DEPENDENCIES.md — dependency justification ledger

Rule (PRD §4 / project governance): adding any dependency not listed in PRD §4
requires one line here plus a justification in the commit body.

Runtime/library deps below are all within PRD §4; the CI-tooling rows are
GitHub Actions / cargo dev tooling (not shipped in the app binary), logged here
for provenance per the project dependency policy.

| Dependency | Where | Why | Added at |
|---|---|---|---|
| `vitest` + `@vitest/coverage-v8` | `package.json` (devDep) | Frontend unit-test runner + v8 coverage for the §15.2 GATE (`pnpm test`); PRD §4 names the React/TS/Vite stack, these are its test layer. | T0.1 |
| `@testing-library/react` + `@testing-library/jest-dom` | `package.json` (devDep) | Render/query React shells and DOM matchers (`toBeInTheDocument`) for `*.test.tsx`. | T0.1 |
| `jsdom` | `package.json` (devDep) | DOM environment for vitest (`environment: jsdom`). | T0.1 |
| `@vitejs/plugin-react` | `package.json` (devDep) | Official Vite↔React integration plugin (JSX/Fast Refresh) — build glue for the §4.1 React+Vite stack. | T0.1 |
| `@tailwindcss/vite` | `package.json` (devDep) | Official Tailwind v4 Vite plugin — the sanctioned §4.1 Tailwind v4 integration. | T0.1 |
| `actions/checkout@v7` | `.github/workflows/{ci,pr-policy}.yml` | First-party checkout action, pinned to `3d3c42e5aac5ba805825da76410c181273ba90b1`; audited from the [official repository](https://github.com/actions/checkout) on 2026-08-05. | T0.0 remediation |
| `actions/setup-node@v7` | `.github/workflows/ci.yml` | First-party Node setup, pinned to `820762786026740c76f36085b0efc47a31fe5020`; audited from the [official repository](https://github.com/actions/setup-node) on 2026-08-05. | T0.0 remediation |
| `pnpm/action-setup@v6` | `.github/workflows/ci.yml` | pnpm setup, pinned to `0977fd99725f1db4007ccb2928dbb4e90d06cc86`; audited from the [official repository](https://github.com/pnpm/action-setup) on 2026-08-05. | T0.0 remediation |
| `dtolnay/rust-toolchain@stable` | `.github/workflows/ci.yml` | Rust toolchain setup, pinned to `4360b52568e2003a75bf9bc1d59f33a8e3fc893c`; audited from the [official repository](https://github.com/dtolnay/rust-toolchain) on 2026-08-05. | T0.0 remediation |
| `Swatinem/rust-cache@v2` | `.github/workflows/ci.yml` | Rust cache, pinned to `e18b497796c12c097a38f9edb9d0641fb99eee32`; audited from the [official repository](https://github.com/Swatinem/rust-cache) on 2026-08-05. | T0.0 remediation |
| `taiki-e/install-action@v2` + `cargo-llvm-cov` | `.github/workflows/ci.yml` | Coverage-tool setup, pinned to `cb33e69fad06166ca28a42b2575e4dadabf62ee8`; audited from the [official repository](https://github.com/taiki-e/install-action) on 2026-08-05. | T0.0 remediation |
| `tempfile` | `src-tauri/Cargo.toml` (dev-dep) | Real tempdirs for `store::settings` filesystem tests (atomic write, permissions, I/O-failure fixtures) per PRD §17.3 "real interfaces, not mocks" — not shipped in the app binary. | T0.3 |
| `async-trait` | `src-tauri/Cargo.toml` | Keeps the async `SpeechRecognizer` boundary usable through the crate's dyn-compatible test/runtime seams on the pinned toolchain; it is not used to claim that async trait syntax is unavailable. | T1.4 |
| `unicode-normalization` | `src-tauri/Cargo.toml` | NFC normalization required by PRD §17.3 WER scoring for deterministic ASR golden comparisons. | T1.6 |
| `@types/react` + `@types/react-dom` | `package.json` (devDeps) | TypeScript declarations for the PRD §4 React 18 frontend; kept explicit for strict `tsc -b` validation. | T0.1 |
| `tauri-plugin-global-shortcut` | `src-tauri/Cargo.toml` | Official Tauri v2 toggle-combo registration required by PRD §4.1 / FR-1.3; the policy manager remains the single action bridge. | T2.1 |
| `objc2` (runtime messaging) + `objc2-app-kit` + `objc2-foundation` | `src-tauri/Cargo.toml` (cfg macos) | Native NSPasteboard access for FR-1.4 clipboard preservation: arboard exposes neither the monotonic `changeCount`, the `public.rtf` flavor, nor a true `clearContents`; all three are required for change detection (R-4), RTF restore, and empty-snapshot clearing. Already vendored transitively via tauri/wry, so declaring them direct does not drift the lockfile. PRD §4 sanctions native AX/Carbon/AVFoundation OS integration; these are its Rust bindings. objc2 also provides the AVFoundation class messaging for the microphone authorization probe. | T2.2 |
