#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  printf 'usage: %s <repository-root>\n' "$0" >&2
  exit 2
fi

repository_root="$1"
if [ ! -d "$repository_root" ]; then
  printf 'repository root is not a directory: %s\n' "$repository_root" >&2
  exit 2
fi

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
hooks_dir="${project_root}/.githooks"
if [ ! -x "${hooks_dir}/pre-commit" ] || [ ! -x "${hooks_dir}/commit-msg" ] || [ ! -x "${hooks_dir}/pre-push" ]; then
  printf 'project hook directory is incomplete: %s\n' "$hooks_dir" >&2
  exit 2
fi

/usr/bin/git -C "$repository_root" rev-parse --is-inside-work-tree >/dev/null 2>&1 || {
  printf 'repository root is not a Git worktree: %s\n' "$repository_root" >&2
  exit 2
}

/usr/bin/git -C "$repository_root" config core.hooksPath "$hooks_dir"
