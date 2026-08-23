#!/bin/sh
# T0.0 remediation regression entrypoint.  The implementation lives beside this
# wrapper so it can use Python's bytes, JSON, and tempfile interfaces without
# sacrificing POSIX-shell invocation on macOS.
set -eu

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
exec python3 "$script_dir/test_t0_0_remediation.py" "$@"
