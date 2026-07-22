# OPEN_QUESTIONS.md — WhisperSpree

Entry format: PRD §16.4. Standing resolutions are read at session start (CLAUDE.md §1).

## Q1 — Private repo vs macOS CI minutes (status: OPEN)
Task: T0.0   PRD: §17.5 / LOOP §6
Context: CI must run on `macos-14` for every PR. The GitHub repo (Muminur/whisperspree) is private; macOS Actions minutes bill at 10× on private repos (free tier ≈ 200 effective macOS-minutes/month). A full M0–M6 delivery will exceed that quota.
Options: A) make the repo public (free standard-runner minutes) B) stay private and accept quota/billing C) stay private with maximal caching, accept risk of CI stalls.
Chosen (conservative): B/C — repo stays private per owner's setup; CI caching maximized. Marker: none (process, not code).
Note (2026-07-22): server-side branch protection on `main` returned 403 (free plan + private repo). PR-only merges are enforced by process (merger agent) until the repo goes public or the plan is upgraded.
Resolution: (owner may flip the repo public at any time to unblock CI quota and enable branch protection)

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
