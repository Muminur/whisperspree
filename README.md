# WhisperSpree

System-wide AI dictation for macOS. Hold a hotkey in **any** app, speak, release — clean, polished text appears in the focused text field.

> **Status: v0.1.0 in development** — foundations underway: the Tauri 2 app shell (menu-bar app, HUD/Settings/History/Onboarding windows), the full CI test gate, the core error taxonomy with redacted rolling-file logging, the settings store (atomic writes, keychain-backed API keys), and the local SQLite history/dictionary/snippet stores with a retention job are in place. Pre-built releases and the one-line installer will ship with the first tagged release. Until then, build from source (below).

## What it does

- **Real-time speech-to-text** — cloud streaming (Deepgram) or fully local (whisper.cpp). Local mode is first-class: everything works offline.
- **Writes like you type, not like you talk** — an AI cleanup pass removes "um/uh", false starts, and self-corrections, and fixes grammar and punctuation.
- **Knows where it's typing** — the frontmost app sets the style: casual for Slack, professional for email, verbatim-safe for code editors and terminals.
- **Programmable** — personal dictionary, spoken snippet macros, custom prompts, tone personas, and 17 transform templates ("turn that into a follow-up email").
- **Multilingual** — ~99 languages with auto-detect and optional live translation.
- **Private by default** — no telemetry, mic opens only while dictating, API keys live in the macOS keychain, no audio stored unless you opt in. Local mode with post-processing off sends zero bytes off the machine.

## Requirements

- macOS 13+ (Apple Silicon or Intel)
- Permissions: Microphone, Accessibility, Input Monitoring (guided onboarding on first run)
- Optional: Deepgram API key (cloud transcription), Anthropic API key (text polish)

## Installation

**Releases:** coming with v0.1.0 — a signed `.dmg` and a one-line installer script will be published on the Releases page.

**Build from source (current):**

```bash
# prerequisites: Xcode CLT, cmake, Rust (stable), Node 22+, pnpm 11+
git clone https://github.com/Muminur/whisperspree.git
cd whisperspree
pnpm i
pnpm tauri dev     # run in development
pnpm tauri build   # produce a release .dmg
```

## Development

- Rust core in `src-tauri/` (audio, ASR, LLM post-processing, injection, storage)
- React + TypeScript windows in `src/` (HUD, Settings, History, Onboarding)
- Full test gate before every commit: `pnpm typecheck && pnpm test` plus `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test` in `src-tauri/`

## License

TBD (pre-release).
