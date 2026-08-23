#!/bin/sh
set -eu

zero_oid='0000000000000000000000000000000000000000'

fail() {
  printf '%s\n' "$1" >&2
  exit 2
}

if [ "$#" -ne 3 ]; then
  fail "usage: $0 <event-name> <event-json> <repository-root>"
fi

event_name="$1"
event_json="$2"
repository_root="$3"

if [ ! -f "$event_json" ] || [ ! -r "$event_json" ]; then
  fail "event payload must be a readable regular file"
fi

if [ ! -d "$repository_root" ] || ! /usr/bin/git -C "$repository_root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  fail "repository root must be a Git worktree"
fi

if ! /usr/bin/python3 - "$event_json" <<'PY'
import json
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as event_file:
        event = json.load(event_file)
except (OSError, UnicodeDecodeError, json.JSONDecodeError):
    sys.exit(1)

sys.exit(0 if type(event) is dict else 1)
PY
then
  fail "event payload must be a JSON object"
fi

metadata_file="$(mktemp "${TMPDIR:-/tmp}/whisperspree-ci-attribution.XXXXXX")"
trap 'rm -f "$metadata_file"' EXIT HUP INT TERM

is_commit_oid() {
  oid="$1"
  printf '%s\n' "$oid" | grep -Eq '^[0-9A-Fa-f]{40}$' &&
    /usr/bin/git -C "$repository_root" cat-file -e "${oid}^{commit}" 2>/dev/null
}

check_metadata_file() {
  "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/check_attribution.sh" "$metadata_file"
}

case "$event_name" in
  pull_request)
    if ! pr_oids="$(/usr/bin/python3 - "$event_json" "$metadata_file" <<'PY'
import json
import re
import sys

event_path, metadata_path = sys.argv[1:]
try:
    with open(event_path, encoding="utf-8") as event_file:
        event = json.load(event_file)
    pull_request = event["pull_request"]
    if type(pull_request) is not dict:
        raise ValueError
    title = pull_request["title"]
    body = pull_request["body"]
    base = pull_request["base"]
    head = pull_request["head"]
    if type(title) is not str or (body is not None and type(body) is not str):
        raise ValueError
    if type(base) is not dict or type(head) is not dict:
        raise ValueError
    base_oid = base["sha"]
    head_oid = head["sha"]
    if type(base_oid) is not str or type(head_oid) is not str:
        raise ValueError
    if not re.fullmatch(r"[0-9A-Fa-f]{40}", base_oid) or not re.fullmatch(r"[0-9A-Fa-f]{40}", head_oid):
        raise ValueError
except (KeyError, OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError):
    sys.exit(1)

with open(metadata_path, "w", encoding="utf-8") as metadata_file:
    metadata_file.write(title)
    metadata_file.write("\n")
    metadata_file.write(body or "")
    metadata_file.write("\n")

print(base_oid)
print(head_oid)
PY
)"; then
      fail "pull_request payload has invalid metadata structure"
    fi

    base_oid="$(printf '%s\n' "$pr_oids" | sed -n '1p')"
    head_oid="$(printf '%s\n' "$pr_oids" | sed -n '2p')"
    if ! is_commit_oid "$base_oid" || ! is_commit_oid "$head_oid"; then
      fail "pull_request payload references an unavailable commit"
    fi
    check_metadata_file
    /usr/bin/git -C "$repository_root" log --format=%B "${base_oid}..${head_oid}" >"$metadata_file"
    check_metadata_file
    ;;
  push)
    if ! push_fields="$(/usr/bin/python3 - "$event_json" <<'PY'
import json
import re
import sys

try:
    with open(sys.argv[1], encoding="utf-8") as event_file:
        event = json.load(event_file)
    before_oid = event["before"]
    after_oid = event["after"]
    ref_name = event["ref"]
    if type(before_oid) is not str or type(after_oid) is not str or type(ref_name) is not str:
        raise ValueError
    if not re.fullmatch(r"[0-9A-Fa-f]{40}", before_oid) or not re.fullmatch(r"[0-9A-Fa-f]{40}", after_oid):
        raise ValueError
    if not ref_name or "\n" in ref_name or "\r" in ref_name:
        raise ValueError
except (KeyError, OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError):
    sys.exit(1)

print(before_oid)
print(after_oid)
print(ref_name)
PY
)"; then
      fail "push payload has invalid metadata structure"
    fi

    before_oid="$(printf '%s\n' "$push_fields" | sed -n '1p')"
    after_oid="$(printf '%s\n' "$push_fields" | sed -n '2p')"
    ref_name="$(printf '%s\n' "$push_fields" | sed -n '3p')"
    case "$ref_name" in
      refs/heads/?* | refs/tags/?*)
        ;;
      *)
        fail "push payload ref must name a branch or tag"
        ;;
    esac
    if ! /usr/bin/git -C "$repository_root" check-ref-format "$ref_name" >/dev/null 2>&1; then
      fail "push payload ref is invalid"
    fi
    if [ "$after_oid" = "$zero_oid" ] || ! is_commit_oid "$after_oid"; then
      fail "push payload after must name an available commit"
    fi
    if [ "$before_oid" = "$zero_oid" ]; then
      /usr/bin/git -C "$repository_root" log --format=%B "$after_oid" >"$metadata_file"
    else
      if ! is_commit_oid "$before_oid"; then
        fail "push payload before must name an available commit"
      fi
      /usr/bin/git -C "$repository_root" log --format=%B "${before_oid}..${after_oid}" >"$metadata_file"
    fi
    check_metadata_file
    ;;
  *)
    fail "unsupported event name: $event_name"
    ;;
esac
