# DEPENDENCIES.md — dependency justification ledger

Rule (CLAUDE.md §7 / PRD §4): adding any dependency not listed in PRD §4
requires one line here plus a justification in the commit body.

Runtime/library deps below are all within PRD §4; the CI-tooling rows are
GitHub Actions / cargo dev tooling (not shipped in the app binary), logged here
for provenance per CLAUDE.md §7.

| Dependency | Where | Why | Added at |
|---|---|---|---|
| `vitest` + `@vitest/coverage-v8` | `package.json` (devDep) | Frontend unit-test runner + v8 coverage for the §15.2 GATE (`pnpm test`); PRD §4 names the React/TS/Vite stack, these are its test layer. | T0.1 |
| `@testing-library/react` + `@testing-library/jest-dom` | `package.json` (devDep) | Render/query React shells and DOM matchers (`toBeInTheDocument`) for `*.test.tsx`. | T0.1 |
| `jsdom` | `package.json` (devDep) | DOM environment for vitest (`environment: jsdom`). | T0.1 |
| `@vitejs/plugin-react` | `package.json` (devDep) | Official Vite↔React integration plugin (JSX/Fast Refresh) — build glue for the §4.1 React+Vite stack. | T0.1 |
| `@tailwindcss/vite` | `package.json` (devDep) | Official Tailwind v4 Vite plugin — the sanctioned §4.1 Tailwind v4 integration. | T0.1 |
| `actions/checkout@v7` | `.github/workflows/ci.yml` (CI tooling) | First-party checkout action for the GATE workflow (§17.5); logged for provenance. | T0.1 |
| `actions/setup-node@v7` | `.github/workflows/ci.yml` (CI tooling) | First-party Node 20 setup + pnpm cache for the GATE workflow (§17.5); logged for provenance. | T0.1 |
| `pnpm/action-setup@v6` | `.github/workflows/ci.yml` (CI tooling) | Installs pnpm on the runner before `setup-node` so the pnpm cache resolves (§17.5). | T0.1 |
| `dtolnay/rust-toolchain@stable` | `.github/workflows/ci.yml` (CI tooling) | Provisions the stable Rust toolchain + `rustfmt`/`clippy`/`llvm-tools-preview` for the GATE (§17.5). | T0.1 |
| `Swatinem/rust-cache@v2` | `.github/workflows/ci.yml` (CI tooling) | Caches `src-tauri/target` to keep macOS CI minutes low (Q1 mitigation). | T0.1 |
| `taiki-e/install-action@v2` + `cargo-llvm-cov` | `.github/workflows/ci.yml` (CI tooling) | Installs the coverage tool; report-only at T0.1, enforced from T0.2 (§17.5). | T0.1 |
