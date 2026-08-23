#!/bin/sh
set -eu
[ "$#" -gt 0 ] || { printf 'attribution checker requires at least one metadata file\n' >&2; exit 2; }
script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
for metadata_file in "$@"; do /usr/bin/python3 "$script_dir/repo_policy.py" attribution "$metadata_file"; done
