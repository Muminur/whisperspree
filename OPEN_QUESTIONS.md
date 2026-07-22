# OPEN_QUESTIONS.md — WhisperSpree

Entry format: PRD §16.4. Standing resolutions are read at session start (CLAUDE.md §1).

## Q1 — Private repo vs macOS CI minutes (status: OPEN)
Task: T0.0   PRD: §17.5 / LOOP §6
Context: CI must run on `macos-14` for every PR. The GitHub repo (Muminur/whisperspree) is private; macOS Actions minutes bill at 10× on private repos (free tier ≈ 200 effective macOS-minutes/month). A full M0–M6 delivery will exceed that quota.
Options: A) make the repo public (free standard-runner minutes) B) stay private and accept quota/billing C) stay private with maximal caching, accept risk of CI stalls.
Chosen (conservative): B/C — repo stays private per owner's setup; CI caching maximized. Marker: none (process, not code).
Resolution: (owner may flip the repo public at any time to unblock CI quota)
