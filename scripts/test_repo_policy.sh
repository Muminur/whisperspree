#!/bin/sh
set -eu

# The managed runner may inject Git controls that the frozen safe-wrapper
# contract deliberately rejects. Re-exec the test harness with only the tools
# it needs on PATH and the two explicitly allowed baseline Git controls.
if [ "${WHISPERSPREE_POLICY_ENV_SANITIZED:-}" != '1' ]; then
  exec env -i \
    PATH="$PATH" \
    PYTHONDONTWRITEBYTECODE=1 \
    GIT_CONFIG_GLOBAL=/dev/null \
    GIT_TERMINAL_PROMPT=0 \
    WHISPERSPREE_POLICY_ENV_SANITIZED=1 \
    sh "$0" "$@"
fi
unset WHISPERSPREE_POLICY_ENV_SANITIZED

# Repository-policy regression suite. All Git interaction is confined to this
# temporary directory and its local bare remotes; it never contacts a network.
rules_file=".codex/rules/default.rules"
repo_root="$(git rev-parse --show-toplevel)"
policy_tmp="$(mktemp -d "${TMPDIR:-/tmp}/whisperspree-policy.XXXXXX")"
zero_oid="0000000000000000000000000000000000000000"

cleanup_policy_tmp() {
  rm -rf "$policy_tmp"
}
trap cleanup_policy_tmp EXIT HUP INT TERM

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

assert_forbidden() {
  policy_output="$(codex execpolicy check --pretty --resolve-host-executables --rules "$rules_file" -- "$@" 2>&1 || true)"
  if ! printf '%s\n' "$policy_output" | grep -q '"decision": "forbidden"'; then
    printf 'expected forbidden policy decision for:' >&2
    printf ' %s' "$@" >&2
    printf '\n%s\n' "$policy_output" >&2
    exit 1
  fi
}

assert_not_forbidden() {
  policy_output="$(codex execpolicy check --pretty --resolve-host-executables --rules "$rules_file" -- "$@" 2>&1 || true)"
  if printf '%s\n' "$policy_output" | grep -q '"decision": "forbidden"'; then
    printf 'expected no categorical policy denial for:' >&2
    printf ' %s' "$@" >&2
    printf '\n%s\n' "$policy_output" >&2
    exit 1
  fi
}

assert_rejected_by_safe_push() {
  if "$repo_root/scripts/safe_git_push.sh" --check-only "$@" >/dev/null 2>&1; then
    printf 'safe push accepted forbidden arguments:' >&2
    printf ' %s' "$@" >&2
    printf '\n' >&2
    exit 1
  fi
}

assert_safe_push_accepted() {
  accepted_repo="$1"
  shift
  if ! (cd "$accepted_repo" && "$accepted_repo/scripts/safe_git_push.sh" --check-only "$@") >/dev/null 2>&1; then
    printf 'safe push rejected allowed arguments:' >&2
    printf ' %s' "$@" >&2
    printf '\n' >&2
    exit 1
  fi
}

init_safe_push_project() {
  safe_push_repo="$1"
  safe_push_branch="$2"
  init_git_repo "$safe_push_repo" "$safe_push_branch"
  mkdir -p "$safe_push_repo/scripts" "$safe_push_repo/.githooks"
  cp "$repo_root/scripts/safe_git_push.sh" "$safe_push_repo/scripts/safe_git_push.sh"
  cp "$repo_root/scripts/repo_policy.py" "$safe_push_repo/scripts/repo_policy.py"
  cp "$repo_root/.githooks/pre-push" "$safe_push_repo/.githooks/pre-push"
  chmod 700 "$safe_push_repo/scripts/safe_git_push.sh" "$safe_push_repo/.githooks/pre-push"
  git -C "$safe_push_repo" remote add origin https://github.com/Muminur/whisperspree.git
}

assert_approval_raw_manifest() {
  approval_repo="$1"
  approval_record="$2"
  python3 -c '
import base64
import json
import subprocess
import sys

repo, record = sys.argv[1:]
raw = subprocess.check_output(
    ["git", "-C", repo, "diff", "--cached", "--raw", "-z", "--no-renames", "--no-abbrev", "--no-ext-diff", "--no-textconv"]
)
expected = []
fields = raw.split(b"\0")
for position in range(0, len(fields) - 1, 2):
    if not fields[position]:
        break
    metadata, status = fields[position][1:].rsplit(b" ", 1)
    _old_mode, mode, _old_oid, blob_oid = metadata.split(b" ")
    if status[:1] != b"D":
        expected.append({"path_b64": base64.b64encode(fields[position + 1]).decode("ascii"), "mode": mode.decode("ascii"), "oid": blob_oid.decode("ascii")})
expected.sort(key=lambda entry: base64.b64decode(entry["path_b64"]))
document = json.load(open(record, encoding="utf-8"))
manifest = document.get("manifest")
if not isinstance(manifest, list) or manifest != expected or any(set(entry) != {"path_b64", "mode", "oid"} for entry in manifest):
    raise SystemExit(1)
' "$approval_repo" "$approval_record"
}

assert_workflow_contains() {
  if ! grep -Fq "$1" "$repo_root/.github/workflows/ci.yml"; then
    printf 'workflow contract is missing: %s\n' "$1" >&2
    exit 1
  fi
}

assert_ci_attribution_accepted() {
  event_name="$1"
  event_json="$2"
  event_repo="$3"
  if ! "$repo_root/scripts/check_ci_attribution.sh" "$event_name" "$event_json" "$event_repo"; then
    printf 'CI attribution checker rejected valid %s event: %s\n' "$event_name" "$event_json" >&2
    exit 1
  fi
}

assert_ci_attribution_rejected() {
  event_name="$1"
  event_json="$2"
  event_repo="$3"
  case_name="$4"
  if "$repo_root/scripts/check_ci_attribution.sh" "$event_name" "$event_json" "$event_repo" >/dev/null 2>&1; then
    printf 'CI attribution checker accepted invalid %s event (%s): %s\n' "$event_name" "$case_name" "$event_json" >&2
    exit 1
  fi
}

write_event() {
  event_file="$1"
  event_body="$2"
  printf '%s\n' "$event_body" >"$event_file"
}

init_git_repo() {
  repo="$1"
  branch_name="${2:-main}"
  git init -q -b "$branch_name" "$repo"
  git -C "$repo" config user.name 'Policy Test'
  git -C "$repo" config user.email 'policy-test@example.invalid'
  printf 'initial fixture\n' >"$repo/fixture.txt"
  git -C "$repo" add fixture.txt
  git -C "$repo" commit -q -m 'test(policy): initial fixture'
}

commit_fixture() {
  repo="$1"
  message="$2"
  printf '%s\n' "$message" >>"$repo/fixture.txt"
  git -C "$repo" add fixture.txt
  git -C "$repo" commit -q -m "$message"
}

init_installed_hook_repo() {
  repo="$1"
  init_git_repo "$repo" 'milestone0/t0-0-staged-policy'

  # The installer is the sole setup path: tests must exercise the actual Git
  # lifecycle rather than configuring core.hooksPath themselves.
  "$repo_root/scripts/install_repo_hooks.sh" "$repo"
  hooks_path="$(git -C "$repo" config --get core.hooksPath || true)"
  if [ "$hooks_path" != "$repo_root/.githooks" ]; then
    fail "install_repo_hooks.sh did not install the project hook set: ${hooks_path}"
  fi
}

assert_commit_rejected() {
  repo="$1"
  message="$2"
  case_name="$3"
  if git -C "$repo" commit -q -m "$message" >/dev/null 2>&1; then
    fail "pre-commit lifecycle accepted ${case_name}"
  fi
}

approve_benign_staged_state() {
  approval_repo="$1"
  approval_rationale="$2"
  "$repo_root/scripts/approve_benign_staged_patterns.sh" "$approval_repo" "$approval_rationale"
}

# Creates a real, neutral base..head commit range for pull_request fixtures.
# The production checker remains solely responsible for parsing the event and
# inspecting the range; this helper only builds local Git test data.
init_neutral_pr_range() {
  repo="$1"
  init_git_repo "$repo"
  pr_base_oid="$(git -C "$repo" rev-parse HEAD)"
  commit_fixture "$repo" 'test(policy): neutral PR range metadata'
  pr_head_oid="$(git -C "$repo" rev-parse HEAD)"
}

write_pr_event() {
  event_file="$1"
  title="$2"
  body_json="$3"
  base_oid="$4"
  head_oid="$5"
  printf '{"pull_request":{"title":"%s","body":%s,"base":{"sha":"%s"},"head":{"sha":"%s"}}}\n' \
    "$title" "$body_json" "$base_oid" "$head_oid" >"$event_file"
}

test_execpolicy_literal_force_push_and_security_matrix() {
  assert_forbidden git push --force origin HEAD
  assert_forbidden git push --force-with-lease origin HEAD
  assert_forbidden git push -f origin HEAD
  assert_not_forbidden git push origin HEAD
  assert_not_forbidden git commit -m test
  assert_not_forbidden gh pr create
  assert_not_forbidden gh pr merge 1
  assert_forbidden git reset --hard HEAD~1
  assert_forbidden git reset HEAD --hard
  assert_forbidden git clean -fdx
  assert_forbidden git clean -fdx -e .keep
  assert_forbidden printenv
  assert_forbidden gh auth token
  assert_forbidden git credential fill
  assert_forbidden security find-generic-password -s whisperspree -w
}

test_execpolicy_does_not_claim_global_option_semantics() {
  # Execpolicy is a literal argv-prefix policy, not a semantic Git parser.
  # Global-option forms remain subject to Auto-review and the repository's
  # safe wrappers rather than being falsely represented as deterministic
  # matches for basename subcommand rules.
  assert_not_forbidden git -c color.ui=false push origin HEAD
  assert_not_forbidden git -C "$repo_root" push origin HEAD
  assert_not_forbidden git -c color.ui=false reset --hard HEAD
  assert_not_forbidden git -c color.ui=false reset HEAD --hard
  assert_not_forbidden git -C "$repo_root" reset --hard HEAD
  assert_not_forbidden git -C "$repo_root" reset HEAD --hard
  assert_not_forbidden git -c color.ui=false clean -fdx
  assert_not_forbidden git -C "$repo_root" clean -fdx -e .keep
  assert_not_forbidden git -c credential.helper=store credential fill
  assert_not_forbidden git -C "$repo_root" credential fill
}

test_execpolicy_does_not_claim_semantic_shell_wrapper_coverage() {
  assert_not_forbidden sh -c 'git push --force origin HEAD'
  assert_not_forbidden command git push origin HEAD
  assert_not_forbidden env git push origin HEAD
  assert_not_forbidden /usr/bin/git push origin HEAD
  assert_forbidden /usr/bin/git reset --hard HEAD
  assert_forbidden /usr/bin/git reset HEAD --hard
  assert_forbidden /usr/bin/git clean -fdx
  assert_forbidden /usr/bin/git credential fill
  assert_forbidden /usr/bin/printenv
  assert_forbidden /usr/bin/security find-internet-password -s whisperspree -w
  assert_forbidden /usr/bin/security dump-keychain
}

test_execpolicy_credential_extraction_variants_are_forbidden() {
  assert_forbidden gh auth status --show-token
  assert_forbidden gh auth status -t
  assert_forbidden gh --hostname github.com auth token
  assert_forbidden gh config get oauth_token
  assert_forbidden security find-internet-password -s whisperspree -w
  assert_forbidden security dump-keychain
}

test_safe_push_rejects_force_delete_broad_and_transport_forms() {
  milestone_ref='HEAD:refs/heads/milestone0/t0-0-policy-contract'
  assert_rejected_by_safe_push --force origin "$milestone_ref"
  assert_rejected_by_safe_push --force-with-lease origin "$milestone_ref"
  assert_rejected_by_safe_push -f origin "$milestone_ref"
  assert_rejected_by_safe_push origin "$milestone_ref" --force
  assert_rejected_by_safe_push origin --force "$milestone_ref"
  assert_rejected_by_safe_push --force=true origin "$milestone_ref"
  assert_rejected_by_safe_push origin -f "$milestone_ref"
  assert_rejected_by_safe_push origin +HEAD:main
  assert_rejected_by_safe_push -d origin refs/heads/milestone0/t0-0-policy-contract
  assert_rejected_by_safe_push --no-verify origin "$milestone_ref"
  assert_rejected_by_safe_push --all origin "$milestone_ref"
  assert_rejected_by_safe_push --tags origin "$milestone_ref"
  assert_rejected_by_safe_push --repo=origin origin "$milestone_ref"
  assert_rejected_by_safe_push --repo origin origin "$milestone_ref"
  assert_rejected_by_safe_push --receive-pack=git-receive-pack origin "$milestone_ref"
  assert_rejected_by_safe_push --receive-pack git-receive-pack origin "$milestone_ref"
  assert_rejected_by_safe_push --exec=git-receive-pack origin "$milestone_ref"
  assert_rejected_by_safe_push --exec git-receive-pack origin "$milestone_ref"
}

test_safe_push_requires_explicit_non_main_milestone_refspec() {
  accepted_repo="$policy_tmp/accepted-safe-push-refspec"
  milestone_ref='HEAD:refs/heads/milestone0/t0-0-policy-contract'
  init_safe_push_project "$accepted_repo" 'milestone0/t0-0-policy-contract'
  assert_rejected_by_safe_push origin
  assert_rejected_by_safe_push origin HEAD:refs/heads/main
  assert_rejected_by_safe_push origin HEAD:main
  assert_rejected_by_safe_push origin main
  assert_safe_push_accepted "$accepted_repo" origin "$milestone_ref"
}

test_safe_push_rejects_config_supplied_refspec() {
  repo="$policy_tmp/config-refspec"
  init_git_repo "$repo"
  git -C "$repo" remote add origin "$policy_tmp/unused-remote.git"
  git -C "$repo" config remote.origin.push 'HEAD:refs/heads/main'
  if (cd "$repo" && "$repo_root/scripts/safe_git_push.sh" --check-only origin >/dev/null 2>&1); then
    fail 'safe push accepted a config-supplied refspec'
  fi
}

test_safe_push_accepts_head_only_from_milestone_branch() {
  milestone_repo="$policy_tmp/milestone-head"
  init_safe_push_project "$milestone_repo" 'milestone0/t0-0-policy-contract'
  if ! (cd "$milestone_repo" && "$milestone_repo/scripts/safe_git_push.sh" --check-only origin HEAD); then
    fail 'safe push rejected HEAD from a mandated milestone branch'
  fi

  nonconforming_repo="$policy_tmp/nonconforming-head"
  init_safe_push_project "$nonconforming_repo" 'feature/policy-contract'
  if (cd "$nonconforming_repo" && "$nonconforming_repo/scripts/safe_git_push.sh" --check-only origin HEAD >/dev/null 2>&1); then
    fail 'safe push accepted HEAD from a nonconforming current branch'
  fi
}

test_attribution_checker_rejects_no_input() {
  if "$repo_root/scripts/check_attribution.sh" >/dev/null 2>&1; then
    fail 'attribution checker accepted no input'
  fi
}

test_attribution_checker_rejects_unreadable_missing_and_nonregular_input() {
  if "$repo_root/scripts/check_attribution.sh" "$policy_tmp/missing-metadata" >/dev/null 2>&1; then
    fail 'attribution checker accepted a missing metadata file'
  fi

  unreadable="$policy_tmp/unreadable-metadata"
  printf 'test(policy): unreadable metadata\n' >"$unreadable"
  chmod 000 "$unreadable"
  if [ ! -r "$unreadable" ] && "$repo_root/scripts/check_attribution.sh" "$unreadable" >/dev/null 2>&1; then
    fail 'attribution checker accepted an unreadable metadata file'
  fi
  chmod 600 "$unreadable"

  metadata_directory="$policy_tmp/metadata-directory"
  mkdir "$metadata_directory"
  if "$repo_root/scripts/check_attribution.sh" "$metadata_directory" >/dev/null 2>&1; then
    fail 'attribution checker accepted a nonregular metadata path'
  fi
}

test_attribution_checker_checks_every_metadata_file() {
  allowed="$policy_tmp/allowed-metadata"
  forbidden="$policy_tmp/later-forbidden-metadata"
  printf 'test(policy): routine metadata\n' >"$allowed"
  printf 'test(policy): AI-assisted metadata\n' >"$forbidden"
  if "$repo_root/scripts/check_attribution.sh" "$allowed" "$forbidden" >/dev/null 2>&1; then
    fail 'attribution checker accepted a later forbidden metadata file'
  fi
}

test_attribution_checker_rejects_every_canonical_forbidden_term() {
  metadata_file="$policy_tmp/forbidden-term-metadata"
  for forbidden_term in Claude Codex OpenAI AI AI-generated AI-assisted 'generated by' 'Co-authored-by:' '🤖'; do
    printf 'test(policy): %s metadata\n' "$forbidden_term" >"$metadata_file"
    if "$repo_root/scripts/check_attribution.sh" "$metadata_file" >/dev/null 2>&1; then
      printf 'attribution checker accepted forbidden term: %s\n' "$forbidden_term" >&2
      exit 1
    fi
  done
}

test_direct_pre_push_hook_fast_forward_non_fast_forward_and_deletion_matrix() {
  head_oid="$(git rev-parse HEAD)"
  parent_oid="$(git rev-parse HEAD^)"

  printf 'refs/heads/test %s refs/heads/test %s\n' "$head_oid" "$parent_oid" |
    "$repo_root/.githooks/pre-push" origin unused

  if printf 'refs/heads/test %s refs/heads/test %s\n' "$parent_oid" "$head_oid" |
    "$repo_root/.githooks/pre-push" origin unused >/dev/null 2>&1; then
    fail 'pre-push hook accepted a non-fast-forward update'
  fi

  if printf 'refs/heads/test %s refs/heads/test %s\n' "$zero_oid" "$head_oid" |
    "$repo_root/.githooks/pre-push" origin unused >/dev/null 2>&1; then
    fail 'pre-push hook accepted a branch deletion'
  fi
}

test_direct_commit_msg_hook_accepts_neutral_and_rejects_attribution() {
  commit_message="$policy_tmp/direct-commit-message"
  printf 'chore(repo): keep governance docs local-only\n' >"$commit_message"
  "$repo_root/.githooks/commit-msg" "$commit_message"

  printf 'chore(repo): AI-assisted bootstrap\n' >"$commit_message"
  if "$repo_root/.githooks/commit-msg" "$commit_message" >/dev/null 2>&1; then
    fail 'commit-msg hook accepted forbidden attribution'
  fi
}

test_ci_attribution_accepts_neutral_real_pr_commit_range() {
  metadata_repo="$policy_tmp/pr-valid"
  init_neutral_pr_range "$metadata_repo"
  string_body="$policy_tmp/pr-valid-string-body.json"
  uppercase_range="$policy_tmp/pr-valid-uppercase-range.json"
  write_pr_event "$string_body" 'test(policy): neutral title' '"neutral body"' "$pr_base_oid" "$pr_head_oid"
  assert_ci_attribution_accepted pull_request "$string_body" "$metadata_repo"

  uppercase_base_oid="$(printf '%s' "$pr_base_oid" | tr '[:lower:]' '[:upper:]')"
  uppercase_head_oid="$(printf '%s' "$pr_head_oid" | tr '[:lower:]' '[:upper:]')"
  write_pr_event "$uppercase_range" 'test(policy): neutral title' '"neutral body"' "$uppercase_base_oid" "$uppercase_head_oid"
  assert_ci_attribution_accepted pull_request "$uppercase_range" "$metadata_repo"
}

test_ci_attribution_accepts_valid_pr_null_body_with_neutral_real_range() {
  metadata_repo="$policy_tmp/pr-valid-null-body"
  init_neutral_pr_range "$metadata_repo"
  null_body="$policy_tmp/pr-valid-null-body.json"
  write_pr_event "$null_body" 'test(policy): neutral title' null "$pr_base_oid" "$pr_head_oid"
  assert_ci_attribution_accepted pull_request "$null_body" "$metadata_repo"
}

test_ci_attribution_rejects_malformed_and_structurally_invalid_pr_events() {
  metadata_repo="$policy_tmp/pr-invalid"
  init_neutral_pr_range "$metadata_repo"
  invalid_file="$policy_tmp/pr-invalid.json"

  write_event "$invalid_file" '{'
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'malformed JSON'
  write_event "$invalid_file" '{}'
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'missing pull_request'
  write_event "$invalid_file" '{"pull_request":[]}'
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'array pull_request'
  write_event "$invalid_file" '{"pull_request":null}'
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'null pull_request'
  write_event "$invalid_file" '{"pull_request":false}'
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'boolean pull_request'
  write_event "$invalid_file" '{"pull_request":"not an object"}'
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'string pull_request'
  write_event "$invalid_file" "{\"pull_request\":{\"body\":null,\"base\":{\"sha\":\"${pr_base_oid}\"},\"head\":{\"sha\":\"${pr_head_oid}\"}}}"
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'missing title'
  for non_string_title in null false 7 '["array"]' '{}'; do
    write_event "$invalid_file" "{\"pull_request\":{\"title\":${non_string_title},\"body\":null,\"base\":{\"sha\":\"${pr_base_oid}\"},\"head\":{\"sha\":\"${pr_head_oid}\"}}}"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "non-string title ${non_string_title}"
  done

  for non_string_body in true 7 '[]' '{}'; do
    write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":${non_string_body},\"base\":{\"sha\":\"${pr_base_oid}\"},\"head\":{\"sha\":\"${pr_head_oid}\"}}}"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "non-null non-string body ${non_string_body}"
  done
}

test_ci_attribution_rejects_forbidden_pr_title_and_body() {
  metadata_repo="$policy_tmp/pr-forbidden"
  init_neutral_pr_range "$metadata_repo"
  forbidden_title="$policy_tmp/pr-forbidden-title.json"
  forbidden_body="$policy_tmp/pr-forbidden-body.json"
  write_pr_event "$forbidden_title" 'test(policy): AI-assisted title' null "$pr_base_oid" "$pr_head_oid"
  write_pr_event "$forbidden_body" 'test(policy): neutral title' '"Generated by Codex"' "$pr_base_oid" "$pr_head_oid"
  assert_ci_attribution_rejected pull_request "$forbidden_title" "$metadata_repo" 'forbidden title'
  assert_ci_attribution_rejected pull_request "$forbidden_body" "$metadata_repo" 'forbidden body'
}

test_ci_attribution_rejects_forbidden_commit_message_in_real_pr_range() {
  metadata_repo="$policy_tmp/pr-forbidden-commit"
  init_neutral_pr_range "$metadata_repo"
  commit_fixture "$metadata_repo" 'test(policy): AI-assisted PR commit metadata'
  pr_head_oid="$(git -C "$metadata_repo" rev-parse HEAD)"
  forbidden_commit="$policy_tmp/pr-forbidden-commit.json"
  write_pr_event "$forbidden_commit" 'test(policy): neutral title' null "$pr_base_oid" "$pr_head_oid"
  assert_ci_attribution_rejected pull_request "$forbidden_commit" "$metadata_repo" 'forbidden commit metadata in PR range'
}

test_ci_attribution_rejects_missing_and_wrong_type_pr_base_head_metadata() {
  metadata_repo="$policy_tmp/pr-invalid-base-head-types"
  init_neutral_pr_range "$metadata_repo"
  invalid_file="$policy_tmp/pr-invalid-base-head-types.json"

  write_event "$invalid_file" '{"pull_request":{"title":"test(policy): neutral title","body":null}}'
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'missing base and head'
  write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"head\":{\"sha\":\"${pr_head_oid}\"}}}"
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'missing base'
  write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"base\":{\"sha\":\"${pr_base_oid}\"}}}"
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'missing head'
  write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"base\":{},\"head\":{\"sha\":\"${pr_head_oid}\"}}}"
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'missing base sha'
  write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"base\":{\"sha\":\"${pr_base_oid}\"},\"head\":{}}}"
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'missing head sha'

  for wrong_base in null '[]' '{}' 7 false; do
    write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"base\":${wrong_base},\"head\":{\"sha\":\"${pr_head_oid}\"}}}"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "wrong-type base ${wrong_base}"
  done
  for wrong_head in null '[]' '{}' 7 false; do
    write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"base\":{\"sha\":\"${pr_base_oid}\"},\"head\":${wrong_head}}}"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "wrong-type head ${wrong_head}"
  done
  for wrong_sha in null '[]' '{}' 7 false; do
    write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"base\":{\"sha\":${wrong_sha}},\"head\":{\"sha\":\"${pr_head_oid}\"}}}"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "wrong-type base sha ${wrong_sha}"
    write_event "$invalid_file" "{\"pull_request\":{\"title\":\"test(policy): neutral title\",\"body\":null,\"base\":{\"sha\":\"${pr_base_oid}\"},\"head\":{\"sha\":${wrong_sha}}}}"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "wrong-type head sha ${wrong_sha}"
  done
}

test_ci_attribution_rejects_invalid_format_and_unavailable_pr_base_head_oids() {
  metadata_repo="$policy_tmp/pr-invalid-base-head-oids"
  init_neutral_pr_range "$metadata_repo"
  invalid_file="$policy_tmp/pr-invalid-base-head-oids.json"
  unavailable_oid='1111111111111111111111111111111111111111'

  for invalid_oid in not-an-oid aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb; do
    write_pr_event "$invalid_file" 'test(policy): neutral title' null "$invalid_oid" "$pr_head_oid"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "invalid-format base oid ${invalid_oid}"
    write_pr_event "$invalid_file" 'test(policy): neutral title' null "$pr_base_oid" "$invalid_oid"
    assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" "invalid-format head oid ${invalid_oid}"
  done

  write_pr_event "$invalid_file" 'test(policy): neutral title' null "$unavailable_oid" "$pr_head_oid"
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'unavailable base oid'
  write_pr_event "$invalid_file" 'test(policy): neutral title' null "$pr_base_oid" "$unavailable_oid"
  assert_ci_attribution_rejected pull_request "$invalid_file" "$metadata_repo" 'unavailable head oid'
}

test_ci_attribution_accepts_valid_branch_and_tag_push_ranges() {
  metadata_repo="$policy_tmp/push-valid"
  init_git_repo "$metadata_repo"
  before_oid="$(git -C "$metadata_repo" rev-parse HEAD)"
  commit_fixture "$metadata_repo" 'test(policy): valid branch push metadata'
  after_oid="$(git -C "$metadata_repo" rev-parse HEAD)"
  branch_event="$policy_tmp/branch-push-valid.json"
  printf '{"before":"%s","after":"%s","ref":"refs/heads/milestone0/t0-0-policy-contract"}\n' "$before_oid" "$after_oid" >"$branch_event"
  assert_ci_attribution_accepted push "$branch_event" "$metadata_repo"

  git -C "$metadata_repo" tag v0.0.0-policy "$after_oid"
  tag_event="$policy_tmp/tag-push-valid.json"
  printf '{"before":"%s","after":"%s","ref":"refs/tags/v0.0.0-policy"}\n' "$zero_oid" "$after_oid" >"$tag_event"
  assert_ci_attribution_accepted push "$tag_event" "$metadata_repo"
}

test_ci_attribution_rejects_forbidden_and_zero_before_push_commit_metadata() {
  metadata_repo="$policy_tmp/push-forbidden"
  init_git_repo "$metadata_repo"
  initial_after="$(git -C "$metadata_repo" rev-parse HEAD)"
  initial_event="$policy_tmp/push-initial-valid.json"
  printf '{"before":"%s","after":"%s","ref":"refs/heads/milestone0/t0-0-policy-contract"}\n' "$zero_oid" "$initial_after" >"$initial_event"
  assert_ci_attribution_accepted push "$initial_event" "$metadata_repo"

  before_oid="$initial_after"
  commit_fixture "$metadata_repo" 'test(policy): AI-assisted push metadata'
  after_oid="$(git -C "$metadata_repo" rev-parse HEAD)"
  forbidden_event="$policy_tmp/push-forbidden.json"
  printf '{"before":"%s","after":"%s","ref":"refs/heads/milestone0/t0-0-policy-contract"}\n' "$before_oid" "$after_oid" >"$forbidden_event"
  assert_ci_attribution_rejected push "$forbidden_event" "$metadata_repo" 'forbidden commit metadata'
}

test_ci_attribution_rejects_invalid_unavailable_and_malformed_push_tag_events() {
  metadata_repo="$policy_tmp/push-invalid"
  init_git_repo "$metadata_repo"
  before_oid="$(git -C "$metadata_repo" rev-parse HEAD)"
  commit_fixture "$metadata_repo" 'test(policy): valid metadata for invalid event tests'
  after_oid="$(git -C "$metadata_repo" rev-parse HEAD)"
  unavailable_oid='1111111111111111111111111111111111111111'
  invalid_file="$policy_tmp/push-invalid.json"
  invalid_ref='refs/notes/policy-test'

  for case_kind in branch tag; do
    if [ "$case_kind" = branch ]; then
      correct_ref='refs/heads/milestone0/t0-0-policy-contract'
    else
      correct_ref='refs/tags/v0.0.0-policy'
    fi
    write_event "$invalid_file" '{'
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push malformed JSON"
    write_event "$invalid_file" "{\"before\":\"${before_oid}\",\"after\":\"${after_oid}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push missing ref"
    write_event "$invalid_file" "{\"before\":\"${before_oid}\",\"after\":\"${after_oid}\",\"ref\":\"${invalid_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push unsupported ref namespace"
    write_event "$invalid_file" "{\"after\":\"${after_oid}\",\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push missing before"
    write_event "$invalid_file" "{\"before\":\"${before_oid}\",\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push missing after"
    write_event "$invalid_file" "{\"before\":[],\"after\":\"${after_oid}\",\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push non-string before"
    write_event "$invalid_file" "{\"before\":\"${before_oid}\",\"after\":{},\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push non-string after"
    write_event "$invalid_file" "{\"before\":\"${before_oid}\",\"after\":\"${after_oid}\",\"ref\":false}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push non-string ref"
    write_event "$invalid_file" "{\"before\":\"not-an-oid\",\"after\":\"${after_oid}\",\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push invalid before OID"
    write_event "$invalid_file" "{\"before\":\"${before_oid}\",\"after\":\"not-an-oid\",\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push invalid after OID"
    write_event "$invalid_file" "{\"before\":\"${unavailable_oid}\",\"after\":\"${after_oid}\",\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push unavailable Git range start"
    write_event "$invalid_file" "{\"before\":\"${before_oid}\",\"after\":\"${unavailable_oid}\",\"ref\":\"${correct_ref}\"}"
    assert_ci_attribution_rejected push "$invalid_file" "$metadata_repo" "${case_kind} push unavailable Git object"
  done
}

test_real_hook_installation_configures_hooks_path_and_invokes_git_lifecycle() {
  bare_repo="$policy_tmp/installed-hooks-remote.git"
  hook_repo="$policy_tmp/installed-hooks"
  git init -q --bare "$bare_repo"
  init_git_repo "$hook_repo" 'milestone0/t0-0-policy-contract'

  # This is intentionally the only installation route: the production entry
  # point must configure the target repository without a test helper doing so.
  "$repo_root/scripts/install_repo_hooks.sh" "$hook_repo"
  hooks_path="$(git -C "$hook_repo" config --get core.hooksPath || true)"
  if [ -z "$hooks_path" ]; then
    fail 'install_repo_hooks.sh did not configure core.hooksPath'
  fi

  printf 'accepted commit\n' >>"$hook_repo/fixture.txt"
  git -C "$hook_repo" add fixture.txt
  git -C "$hook_repo" commit -q -m 'test(policy): installed commit hook accepts neutral metadata'

  printf 'rejected commit\n' >>"$hook_repo/fixture.txt"
  git -C "$hook_repo" add fixture.txt
  if git -C "$hook_repo" commit -q -m 'test(policy): AI-assisted installed hook metadata' >/dev/null 2>&1; then
    fail 'installed commit-msg hook accepted forbidden attribution'
  fi
  git -C "$hook_repo" reset -q --mixed HEAD

  git -C "$hook_repo" remote add origin "$bare_repo"
  git -C "$hook_repo" push -q origin HEAD:refs/heads/milestone0/t0-0-policy-contract
  if git -C "$hook_repo" push -q origin :refs/heads/milestone0/t0-0-policy-contract >/dev/null 2>&1; then
    fail 'installed pre-push hook accepted a branch deletion'
  fi
  if ! git --git-dir="$bare_repo" show-ref --verify --quiet refs/heads/milestone0/t0-0-policy-contract; then
    fail 'installed pre-push hook allowed deletion at the local bare remote'
  fi
}

# T0.0 staged-policy contracts. These tests intentionally precede the older
# execpolicy RED: a missing pre-commit hook is the first production contract
# this remediation is expected to make fail.
test_staged_policy_installed_hook_set_requires_executable_pre_commit() {
  if [ ! -x "$repo_root/.githooks/pre-commit" ]; then
    fail 'installed hook set requires an executable .githooks/pre-commit'
  fi
}

test_staged_policy_production_contracts_are_executable() {
  for production_contract in \
    "$repo_root/scripts/check_staged_policy.sh" \
    "$repo_root/scripts/approve_benign_staged_patterns.sh" \
    "$repo_root/.githooks/pre-commit"; do
    if [ ! -x "$production_contract" ]; then
      fail "staged-policy production contract is missing or not executable: ${production_contract#$repo_root/}"
    fi
  done
}

test_staged_policy_installer_accepts_neutral_real_staged_commit() {
  hook_repo="$policy_tmp/staged-policy-neutral"
  init_installed_hook_repo "$hook_repo"

  printf 'neutral staged content\n' >"$hook_repo/neutral.txt"
  git -C "$hook_repo" add neutral.txt
  git -C "$hook_repo" commit -q -m 'test(policy): neutral staged policy commit'
}

test_staged_policy_real_pre_commit_rejects_governance_path_without_approval() {
  hook_repo="$policy_tmp/staged-policy-governance-reject"
  init_installed_hook_repo "$hook_repo"

  printf '# local-only governance fixture\n' >"$hook_repo/AGENTS.md"
  git -C "$hook_repo" add -f AGENTS.md
  assert_commit_rejected "$hook_repo" 'test(policy): governance path must be rejected' 'a staged local-only governance path'
}

test_staged_policy_real_pre_commit_rejects_loop_7_secret_pattern_without_approval() {
  hook_repo="$policy_tmp/staged-policy-secret-reject"
  init_installed_hook_repo "$hook_repo"

  # This is a deliberately non-credential LOOP §7 marker. It tests the
  # scanner's policy behavior without placing a secret value in the fixture.
  printf 'OPENAI_API_KEY=LOOP7_BENIGN_TEST_MARKER\n' >"$hook_repo/loop7-marker.txt"
  git -C "$hook_repo" add loop7-marker.txt
  assert_commit_rejected "$hook_repo" 'test(policy): unreviewed LOOP 7 marker must be rejected' 'a staged LOOP §7 secret-pattern marker without explicit review'
}

test_staged_policy_real_pre_commit_rejects_every_required_secret_marker_family_without_approval() {
  # Every marker is deliberately non-credential test text. Each iteration uses
  # a fresh repository and the production installer to exercise Git's actual
  # installed pre-commit lifecycle.
  for marker_family in \
    api_key \
    secret \
    token \
    password \
    sk_ant \
    sk_proj \
    sk_svcacct \
    ghp \
    gho \
    ghu \
    ghs \
    ghr \
    github_pat \
    private_key; do
    case "$marker_family" in
      api_key) marker_text='API_KEY=POLICY_TEST_MARKER' ;;
      secret) marker_text='SECRET=POLICY_TEST_MARKER' ;;
      token) marker_text='TOKEN=POLICY_TEST_MARKER' ;;
      password) marker_text='PASSWORD=POLICY_TEST_MARKER' ;;
      sk_ant) marker_text='sk-ant-POLICY_TEST_MARKER' ;;
      sk_proj) marker_text='sk-proj-POLICY_TEST_MARKER' ;;
      sk_svcacct) marker_text='sk-svcacct-POLICY_TEST_MARKER' ;;
      ghp) marker_text='ghp_POLICY_TEST_MARKER' ;;
      gho) marker_text='gho_POLICY_TEST_MARKER' ;;
      ghu) marker_text='ghu_POLICY_TEST_MARKER' ;;
      ghs) marker_text='ghs_POLICY_TEST_MARKER' ;;
      ghr) marker_text='ghr_POLICY_TEST_MARKER' ;;
      github_pat) marker_text='github_pat_POLICY_TEST_MARKER' ;;
      private_key) marker_text='BEGIN BENIGN PRIVATE KEY' ;;
    esac

    hook_repo="$policy_tmp/staged-policy-marker-family-${marker_family}"
    init_installed_hook_repo "$hook_repo"
    printf '%s\n' "$marker_text" >"$hook_repo/policy-marker.txt"
    git -C "$hook_repo" add policy-marker.txt
    assert_commit_rejected "$hook_repo" 'test(policy): unreviewed staged policy marker must be rejected' 'a staged policy marker without explicit review'
  done
}

test_staged_policy_self_issued_approval_never_authorizes_a_finding() {
  hook_repo="$policy_tmp/staged-policy-approval"
  rationale='benign policy fixture: environment-name marker, not a credential'
  init_installed_hook_repo "$hook_repo"

  printf 'OPENAI_API_KEY=LOOP7_BENIGN_TEST_MARKER\n' >"$hook_repo/loop7-marker.txt"
  git -C "$hook_repo" add loop7-marker.txt
  if approve_benign_staged_state "$hook_repo" '' >/dev/null 2>&1; then
    fail 'self-issued staged approval accepted an empty rationale'
  fi
  if approve_benign_staged_state "$hook_repo" "$rationale" >/dev/null 2>&1; then
    fail 'self-issued staged approval accepted a nonempty rationale'
  fi

  target_git_dir="$(git -C "$hook_repo" rev-parse --absolute-git-dir)"
  approval_record="$target_git_dir/whisperspree-staged-policy-approval"
  if [ -e "$approval_record" ] || [ -L "$approval_record" ]; then
    fail 'self-issued staged approval created an authorization artifact'
  fi
  assert_commit_rejected "$hook_repo" 'test(policy): self-issued approval must not permit marker' 'a LOOP §7 marker after attempted self-issued approval'
}

test_staged_policy_governance_paths_remain_unapprovable() {
  hook_repo="$policy_tmp/staged-policy-governance-unapprovable"
  init_installed_hook_repo "$hook_repo"

  printf '# governance path remains local only\n' >"$hook_repo/PLANNING.md"
  git -C "$hook_repo" add -f PLANNING.md
  if approve_benign_staged_state "$hook_repo" 'attempted governance-path exception' >/dev/null 2>&1; then
    fail 'benign staged-pattern approval accepted a local-only governance path'
  fi
  assert_commit_rejected "$hook_repo" 'test(policy): governance path remains unapprovable' 'a governance path after attempted approval'
}

test_workflow_delegates_ci_attribution_and_uses_read_only_checkout() {
  assert_workflow_contains 'scripts/check_ci_attribution.sh "${GITHUB_EVENT_NAME}" "${GITHUB_EVENT_PATH}" "${GITHUB_WORKSPACE}"'
  assert_workflow_contains 'permissions:'
  assert_workflow_contains 'contents: read'
  assert_workflow_contains 'persist-credentials: false'
}

test_workflow_frontend_coverage_enforced_contract() {
  expected_step_name='Frontend coverage (enforced ≥85%)'
  expected_command='pnpm exec vitest run --coverage'
  workflow_file="$repo_root/.github/workflows/ci.yml"

  # Bind the exact command to the named coverage step in the GATE job, rather
  # than accepting the command if it appears in an unrelated workflow step.
  if ! awk -v expected_step_name="$expected_step_name" -v expected_command="$expected_command" '
    $0 == "  gate:" {
      in_gate_job = 1
      in_frontend_coverage_step = 0
      next
    }
    in_gate_job && /^  [[:alnum:]_-]+:$/ {
      in_gate_job = 0
      in_frontend_coverage_step = 0
      next
    }
    in_gate_job && $0 == "      - name: " expected_step_name {
      frontend_coverage_step_count += 1
      in_frontend_coverage_step = 1
      next
    }
    in_gate_job && /^      - name: / {
      in_frontend_coverage_step = 0
      next
    }
    in_gate_job && in_frontend_coverage_step && /^        run:/ {
      frontend_coverage_run_count += 1
      if ($0 == "        run: " expected_command) {
        expected_frontend_coverage_run_count += 1
      }
    }
    END {
      exit !(frontend_coverage_step_count == 1 && frontend_coverage_run_count == 1 && expected_frontend_coverage_run_count == 1)
    }
  ' "$workflow_file"; then
    printf 'workflow contract is missing: GATE step %s with exact enforced command: %s\n' \
      "$expected_step_name" "$expected_command" >&2
    exit 1
  fi

  if grep -Fq 'pnpm test -- --coverage' "$workflow_file"; then
    printf 'workflow contract contains ineffective frontend coverage command: pnpm test -- --coverage\n' >&2
    exit 1
  fi
}

# The staged-policy hook contract is the designated deterministic first RED.
# The canonical baseline, including existing execpolicy coverage, follows.
test_staged_policy_installed_hook_set_requires_executable_pre_commit
test_staged_policy_production_contracts_are_executable
test_staged_policy_installer_accepts_neutral_real_staged_commit
test_staged_policy_real_pre_commit_rejects_governance_path_without_approval
test_staged_policy_real_pre_commit_rejects_loop_7_secret_pattern_without_approval
test_staged_policy_real_pre_commit_rejects_every_required_secret_marker_family_without_approval
test_staged_policy_self_issued_approval_never_authorizes_a_finding
test_staged_policy_governance_paths_remain_unapprovable

test_execpolicy_does_not_claim_global_option_semantics
test_execpolicy_literal_force_push_and_security_matrix
test_execpolicy_does_not_claim_semantic_shell_wrapper_coverage
test_execpolicy_credential_extraction_variants_are_forbidden
test_workflow_frontend_coverage_enforced_contract
test_safe_push_rejects_force_delete_broad_and_transport_forms
test_safe_push_requires_explicit_non_main_milestone_refspec
test_safe_push_rejects_config_supplied_refspec
test_safe_push_accepts_head_only_from_milestone_branch
test_attribution_checker_rejects_no_input
test_attribution_checker_rejects_unreadable_missing_and_nonregular_input
test_attribution_checker_checks_every_metadata_file
test_attribution_checker_rejects_every_canonical_forbidden_term
test_direct_pre_push_hook_fast_forward_non_fast_forward_and_deletion_matrix
test_direct_commit_msg_hook_accepts_neutral_and_rejects_attribution
test_ci_attribution_accepts_neutral_real_pr_commit_range
test_ci_attribution_accepts_valid_pr_null_body_with_neutral_real_range
test_ci_attribution_rejects_malformed_and_structurally_invalid_pr_events
test_ci_attribution_rejects_forbidden_pr_title_and_body
test_ci_attribution_rejects_forbidden_commit_message_in_real_pr_range
test_ci_attribution_rejects_missing_and_wrong_type_pr_base_head_metadata
test_ci_attribution_rejects_invalid_format_and_unavailable_pr_base_head_oids
test_ci_attribution_accepts_valid_branch_and_tag_push_ranges
test_ci_attribution_rejects_forbidden_and_zero_before_push_commit_metadata
test_ci_attribution_rejects_invalid_unavailable_and_malformed_push_tag_events
test_real_hook_installation_configures_hooks_path_and_invokes_git_lifecycle
test_workflow_delegates_ci_attribution_and_uses_read_only_checkout

printf 'repository policy tests passed\n'
