#!/bin/sh
set -eu
[ "$#" -eq 3 ] || [ "$#" -eq 5 ] || { printf 'usage: %s <event-name> <event-json> <repository-root> [--exceptions file]\n' "$0" >&2; exit 2; }
script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
exec /usr/bin/python3 "$script_dir/repo_policy.py" ci "$@"
