#!/bin/sh
set -eu
[ "$#" -eq 1 ] || { printf 'usage: %s <repository-root>\n' "$0" >&2; exit 2; }
script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
exec /usr/bin/python3 "$script_dir/repo_policy.py" staged "$1"
