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

## Q9 — §11 does not enumerate the four lookup-table stores (status: RESOLVED)
Task: T0.4   PRD: §11 / §8.1
Context: §11's condensed store list names only `db.rs`, `settings.rs`, `keychain.rs`, `history.rs`, but §8.1 defines seven tables — `dictionary_entries`, `snippets`, `custom_prompts` and `app_rules` have no file assigned. A separate top-level `src-tauri/src/dictionary.rs` (FR-3.1 glossary/stoplist logic) is planned for T5.1 and is a DIFFERENT module from the `dictionary_entries` CRUD store; identically-named modules would invite confusion.
Options: A) fold all four into `history.rs` or a single `lookups.rs` B) one file per table under `store/`, mirroring the `settings.rs`/`keychain.rs` precedent.
Chosen (conservative): B — `store/dictionary_entries.rs`, `store/snippets.rs`, `store/custom_prompts.rs`, `store/app_rules.rs`. Deliberately `dictionary_entries.rs`, not `dictionary.rs`, to keep it distinct from the future FR-3.1 module. All live under `store::` so `cargo test store::` still filters correctly. Marker: none (layout choice, recorded here).
Resolution: revisit only if a later PRD version enumerates §11 fully.

## Q10 — §8.1 leaves the schema_version bump policy undefined (status: RESOLVED)
Task: T0.4   PRD: §8.1
Context: §8.1 makes `schema_version` a TABLE seeded by `0001_init.sql` itself (`INSERT INTO schema_version VALUES (1)`), and states migrations are forward-only files applied when numbered `> schema_version`. It never says how the version is bumped for `NNNN > 1`, and the table has no PRIMARY KEY or single-row constraint, so nothing prevents multiple rows accumulating.
Options: A) each migration file self-seeds its own version (as `0001` does) B) the runner owns the bump after applying each file.
Chosen (conservative): B — the runner sets the version after each applied file, inside the same transaction. For `0001` this is redundant-but-idempotent since the file self-seeds. The runner always reads `SELECT MAX(version)`, never `SELECT version`, so a multi-row table cannot silently break version detection. A DB whose version exceeds the highest known migration is left untouched and returns Ok (forward-compatible: a newer app wrote it), never downgraded. Marker: `// PRD-QUESTION(Q10)` at the migration runner.
Resolution: pinned by tests `db::migrate_applies_and_sets_version_1`, `db::migrate_is_idempotent`, `db::migrate_future_version_left_untouched`. Future migration authors must NOT self-seed a version row.

## Q11 — §8.1 does not define what retentionDays = 0 means (status: RESOLVED)
Task: T0.4   PRD: §8.1 / §8.3
Context: §8.1's retention job deletes dictations older than `history.retentionDays`. §8.3 defaults it to 30 and validation clamps the floor to 0, but neither section says whether 0 means "retain nothing" (delete everything immediately) or "retention disabled".
Options: A) 0 = delete all history B) 0 = retention disabled, no-op.
Chosen (conservative): B — `retention_sweep` returns immediately with no deletions when `retention_days == 0`. Destroying a user's entire history on an off-by-one or an unset field is unrecoverable; declining to delete is not. Marker: `// PRD-QUESTION(Q11)` at `retention_sweep`.
Resolution: pinned by test `history::retention_days_zero_is_noop`; revisit if the PRD later defines 0 explicitly or the UI offers a "keep nothing" option.

## Q12 — `PRAGMA journal_mode` cannot run inside a transaction, so migration text is filtered (status: RESOLVED)
Task: T0.4   PRD: §8.1
Context: §8.1's `0001_init.sql` begins with `PRAGMA journal_mode = WAL;` and `PRAGMA foreign_keys = ON;`, but the §8.1 migration policy requires each migration to be applied inside a transaction. SQLite refuses `journal_mode` changes inside an open transaction ("cannot change into wal mode from within a transaction"), so executing the file's raw text in a transaction fails on migration 1 every time. Neither §8.1 nor the T0.4 plan anticipated this.
Options: A) run migrations outside a transaction (loses atomicity — a partially-applied migration would persist) B) keep the transaction and filter PRAGMA lines out of the *executed* text, leaving the on-disk `.sql` byte-identical to §8.1.
Chosen (conservative): B — atomicity of migrations matters more than executing two statements that `db::open` already applies per-connection. The on-disk migration file stays byte-identical to §8.1 for spec fidelity; only the in-memory text is filtered. Nothing is lost: `journal_mode`/`foreign_keys` are applied by `db::open` on every connection, which is strictly stronger than relying on the migration (which runs only once, on a fresh DB). Marker: `// PRD-QUESTION(Q12)` at the filter helper in `store/db.rs`.
Resolution: **Standing rule for future migration authors** — a migration file must not depend on a PRAGMA taking effect, because PRAGMA lines are stripped before execution. In particular the common SQLite table-rebuild pattern that wraps work in `PRAGMA foreign_keys=OFF` … `PRAGMA foreign_keys=ON` would be silently defeated. The filter therefore accepts ONLY `journal_mode` and `foreign_keys` and returns an error for any other PRAGMA, so a future migration relying on one fails loudly at apply time instead of corrupting data silently.
