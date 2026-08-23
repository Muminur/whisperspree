#!/bin/sh
set -eu
forbidden_policy_environment() {
  [ "${DYLD_INSERT_LIBRARIES+x}" = x ] || [ "${LD_PRELOAD+x}" = x ] || \
    [ "${PYTHONPATH+x}" = x ] || [ "${PYTHONHOME+x}" = x ] || \
    [ "${PYTHONSTARTUP+x}" = x ] || [ "${BASH_ENV+x}" = x ] || \
    [ "${ENV+x}" = x ] || [ "${ZDOTDIR+x}" = x ]
}
if forbidden_policy_environment; then
  printf 'unsafe policy environment\n' >&2
  exit 2
fi
check_only=0
if [ "${1:-}" = "--check-only" ]; then check_only=1; shift; fi
script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
project_root="$(CDPATH= cd -- "$script_dir/.." && pwd)"
exec /usr/bin/python3 "$script_dir/repo_policy.py" push "$project_root" "$check_only" "$@"
