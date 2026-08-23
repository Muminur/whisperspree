#!/bin/sh
set -eu
[ "$#" -eq 1 ] || { printf 'safe commit requires exactly one message argument\n' >&2; exit 2; }
script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
project_root="$(CDPATH= cd -- "$script_dir/.." && pwd)"
exec python3 "$script_dir/repo_policy.py" commit "$project_root" "$1"
