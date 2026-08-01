# OPEN_QUESTIONS.md — WhisperSpree

Entry format: PRD §16.4. Standing resolutions are read at session start (CLAUDE.md §1).

## Q1 — Private repo vs macOS CI minutes (status: RESOLVED)
Task: T0.0   PRD: §17.5 / LOOP §6
Context: CI must run on `macos-14` for every PR. The GitHub repo (Muminur/whisperspree) is private; macOS Actions minutes bill at 10× on private repos (free tier ≈ 200 effective macOS-minutes/month). A full M0–M6 delivery will exceed that quota.
Options: A) make the repo public (free standard-runner minutes) B) stay private and accept quota/billing C) stay private with maximal caching, accept risk of CI stalls.
Chosen (conservative): B/C — repo stays private per owner's setup; CI caching maximized. Marker: none (process, not code).
Note (2026-07-22): server-side branch protection on `main` returned 403 (free plan + private repo). PR-only merges are enforced by process (merger agent) until the repo goes public or the plan is upgraded.
Resolution: 2026-07-23 — first CI run was blocked by the billing/spending limit (macOS jobs never started). Owner chose to make the repo PUBLIC **temporarily**: revert to PRIVATE after v0.1.0 delivery (owner instruction, 2026-07-23; tracked as a post-R6 step). Visibility flipped, branch protection enabled on main (required check: the GATE job; force-pushes and deletions blocked), CI re-run.

## Q5 — CI Node version: LOOP §6 says Node 20, pnpm 11 needs ≥22.13 (status: RESOLVED)
Task: T0.1   PRD: §4.1 / §17.5 (PLANNING §6: "Node.js 20+")
Context: The repo's pinned package manager (packageManager: pnpm@11.15.1, required by the local toolchain) uses the node:sqlite builtin and refuses Node < 22.13. CI on Node 20 fails at "Set up Node" (ERR_UNKNOWN_BUILTIN_MODULE, observed run 29957398582).
Options: A) CI matrix Node 22 LTS (PRD "20+" permits it) B) downgrade pnpm to 10 (risks lockfile/toolchain drift vs the dev machine).
Chosen (conservative): A — CI runs Node 22; dev machine runs 24; PRD floor stays 20+. Marker: none (CI config).
Resolution: Node 22 in the CI matrix; revisit only if PRD pins an exact version.

## Q6 — §14 has no explicit `recoverable` column; mapping derived (status: RESOLVED)
Task: T0.2   PRD: §14 / §9.2 (`app:error {recoverable}`)
Context: The `app:error` event carries `recoverable: bool`, but the §14 matrix has no recoverable column. Derived from the §14 rule ("recoverable errors never crash the session task") plus each row's Recovery column: a code is recoverable when the session continues/degrades without user unblocking.
Options: A) all errors recoverable B) per-row derivation.
Chosen (conservative): B — recoverable:true = MIC-DEV, ASR-SLOW, NET-STREAM, LLM-AUTH, LLM-TIMEOUT, LLM-VERIFY, INJ-FAIL, DB-IO, SEC-FIELD, AX-PERM (continues via clipboard_only); recoverable:false = MIC-PERM, HK-PERM, ASR-NOMODEL, ASR-LOAD. Marker: // PRD-QUESTION(Q6) at the mapping in error.rs.
Resolution: pinned by tests error::recoverable_flag_matches_matrix; revisit only if a later PRD version adds the column.

## Q7 — Tracing bootstrap placement vs the 85% coverage gate (status: RESOLVED)
Task: T0.2   PRD: §11 / §12 P-3 / §15.2
Context: §11 has no dedicated logging module. init_tracing() installs a global subscriber, spawns the appender worker thread, and touches the real FS — headless-untestable bootstrap glue. Housed in error.rs it drags the file to 84% and fails the 85% gate.
Options: A) bootstrap fns live in lib.rs (already in the sanctioned coverage-ignore set as GUI/bootstrap glue) B) padding tests in error.rs C) threshold carve-out.
Chosen (conservative): A — init_tracing/log_dir are private fns in lib.rs; error.rs keeps the pure, fully-tested logic (taxonomy, redact(), RedactingWriter). Marker: comment at the fns in lib.rs.
Resolution: standing rule — headless-untestable bootstrap lives in the ignored bootstrap files; the coverage gate stays at 85 with no carve-outs for logic files.

## Q2 — PRD cites §17.6 for CI; the CI spec is §17.5 (status: RESOLVED)
Task: T0.1   PRD: §15.3 / §17.5
Context: The §15.3 T0.1 row says "CI workflow §17.6" but §17 has no .6 — the CI spec is §17.5. TASKS.md T0.1 already says §17.5.
Options: A) treat as typo, follow §17.5 B) assume a missing section.
Chosen (conservative): A — follow §17.5. Marker: none needed (docs-only typo).
Resolution: §17.5 is the CI spec; PRD reference is a typo.

## Q3 — HUD click-through / non-activating depth at scaffold time (status: RESOLVED)
Task: T0.1   PRD: FR-5.2 / §9.3
Context: FR-5.2 requires the HUD to be click-through except its buttons and never steal focus. Tauri window config offers declarative `focusable:false`/`transparent`/`alwaysOnTop`/`visibleOnAllWorkspaces`, but true click-through (ignore-mouse-events with per-element exceptions) needs native NSWindow calls.
Options: A) native NSWindow work in T0.1 B) declarative flags in T0.1, native click-through in T2.4 behind the §9.3 macOS module.
Chosen (conservative): B — T0.1 sets the declarative flags only; T2.4 (HUD feature task) owns native click-through. Marker: // PRD-QUESTION(Q3) in tauri.conf.json comment is impossible (JSON) — recorded here instead.
Resolution: R0 must not fail T0.1 for missing native click-through; T2.4 acceptance covers it.

## Q4 — Keychain access in tests (status: RESOLVED)
Task: T0.3   PRD: §4.2 / §12 P-3 / CLAUDE.md §3
Context: `keyring` hits the real macOS keychain, which can prompt or fail headless — CI cannot grant that permission. Keychain is an OS-permission surface under CLAUDE.md §3.
Options: A) real keychain in tests B) `KeyStore` trait with in-memory double by default; real keyring impl exercised only by an `#[ignore]` opt-in test.
Chosen (conservative): B. Marker: // PRD-QUESTION(Q4) at the KeyStore trait definition.
Resolution: standing test strategy for all keychain-touching code.

## Q8 — §14 has no code for settings-file/keychain I/O or invalid settings patches (status: RESOLVED)
Task: T0.3   PRD: §14 / §8.3 / §9.1 / §12 P-3
Context: T0.3 raises three error paths the closed 14-code §14 matrix does not name: (a) settings.json read/write I/O failure, (b) keychain set/get/delete failure, (c) `update_settings` receiving an out-of-range or malformed value. `DB-IO` is described as "sqlite failure"; `SEC-FIELD` is "secure input active" — neither is an obvious fit, and adding a 15th code would break the pinned §14 code test.
Options: A) invent a new `SET-IO` / `KEY-IO` code B) map (a)+(b) to `DB-IO` by broadening it to "local persistence"; treat (c) as validation, not an error.
Chosen (conservative): B — (a)+(b) → `DB-IO` (the only local-persistence-failure code; its recovery column "non-blocking toast" fits; recoverable=true per Q6). (c) is not an error: `validate()` clamps numerics and drops invalid enums/accelerators, keeping the prior valid value, and `update_settings` returns the resulting `Settings` — mirrors §8.3 "unknown fields preserved on rewrite". A bad patch never corrupts the file — but note `validate()` alone does NOT achieve that: it guards enums and numeric floors, not field *types*, so `update()` must additionally prove the merged value round-trips (`serde_json::from_value`) **before** `atomic_write` touches disk. Without that guard a patch like `{"version":"notanumber"}` is persisted and then permanently breaks every later `get_settings` with `DB-IO`. Marker: `// PRD-QUESTION(Q8)` at the `DbIo` map helper in `store/mod.rs` (a+b) and at `validate()` in `store/settings.rs` (c).
Resolution: pinned by tests `settings::settings_write_io_failure_maps_db_io`, `keychain::keystore_failure_maps_db_io`, `settings::validate_clamps_timeout_ms`, `settings::validate_rejects_invalid_hotkey_keeps_previous`, and — for the no-corruption invariant specifically — `settings::update_type_invalid_patch_does_not_corrupt_file` / `settings::update_type_invalid_nested_patch_does_not_corrupt_file`; revisit if a later PRD version adds a persistence/validation code.
