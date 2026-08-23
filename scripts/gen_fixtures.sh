#!/usr/bin/env bash
# Generate the checked-in ASR fixtures from PRD §17.2. Requires macOS `say`,
# `afconvert`, and `python3` (see PLANNING.md §6).
set -euo pipefail

cd "$(dirname "$0")/../fixtures"

gen() {
  say -v "$1" -o /tmp/ws_fix.aiff "$3"
  afconvert -f WAVE -d LEI16@16000 -c 1 /tmp/ws_fix.aiff "$2"
}

gen "Samantha" f1_short.wav "Hello world, this is a short dictation test."
gen "Samantha" f2_long.wav "Okay so um I wanted to to talk about the quarterly report, actually no wait, the monthly report, and make sure that we um send it to the whole team by Friday afternoon, you know, before the deadline."
gen "Mónica" f3_es.wav "Hola, esto es una prueba de dictado en español para WhisperSpree."

python3 - <<'PY'
import struct
import wave

with wave.open("silence_2s.wav", "w") as output:
    output.setnchannels(1)
    output.setsampwidth(2)
    output.setframerate(16000)
    output.writeframes(struct.pack("<h", 0) * 32000)
PY
