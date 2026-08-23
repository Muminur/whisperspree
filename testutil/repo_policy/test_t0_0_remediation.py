#!/usr/bin/env python3
"""Executable red regressions for the T0.0 repository-policy remediation.

This suite is intentionally self-contained.  It creates only disposable local
Git repositories and bare remotes, invokes the checked-in policy interfaces as
real subprocesses, and never prints the benign policy marker used in blobs.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable


ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
ZERO_OID = "0" * 40
CANONICAL_ORIGIN = "https://github.com/Muminur/whisperspree.git"
# This is deliberately only a benign detector marker.  It is never included in
# diagnostics, output, commit messages, or fixture filenames.
MARKER = b"POLICY_TOKEN_REGRESSION_MARKER_42"


def sanitized_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    # Every subprocess starts from a deterministic, non-controlling Git
    # environment. Individual regressions may then inject exactly one hostile
    # variable and prove that the wrapper rejects it.
    env = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
    env["GIT_CONFIG_GLOBAL"] = "/dev/null"
    env["GIT_TERMINAL_PROMPT"] = "0"
    if extra:
        env.update(extra)
    return env


def run(
    argv: list[str | Path],
    *,
    cwd: Path | None = None,
    extra_env: dict[str, str] | None = None,
    input_bytes: bytes | None = None,
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [str(item) for item in argv],
        cwd=str(cwd) if cwd else None,
        env=sanitized_env(extra_env),
        input=input_bytes,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def must_run(argv: list[str | Path], *, cwd: Path | None = None) -> bytes:
    result = run(argv, cwd=cwd)
    if result.returncode:
        raise RuntimeError("fixture setup command failed: " + " ".join(map(str, argv)))
    return result.stdout


def require(condition: bool, explanation: str) -> None:
    if not condition:
        raise AssertionError(explanation)


def git(repo: Path, *args: str, input_bytes: bytes | None = None) -> subprocess.CompletedProcess[bytes]:
    return run(["git", "-C", repo, *args], input_bytes=input_bytes)


def git_ok(repo: Path, *args: str, input_bytes: bytes | None = None) -> bytes:
    result = git(repo, *args, input_bytes=input_bytes)
    if result.returncode:
        raise RuntimeError("Git fixture setup failed: " + " ".join(args))
    return result.stdout


def oid(repo: Path, rev: str = "HEAD") -> str:
    return git_ok(repo, "rev-parse", rev).decode().strip()


def init_repo(root: Path, name: str, branch: str = "milestone0/t0-0-remediation") -> Path:
    repo = root / name
    must_run(["git", "init", "-q", "-b", branch, repo])
    git_ok(repo, "config", "user.name", "Policy Regression")
    git_ok(repo, "config", "user.email", "policy-regression@example.invalid")
    (repo / "base.txt").write_text("base\n", encoding="utf-8")
    git_ok(repo, "add", "base.txt")
    git_ok(repo, "commit", "-q", "-m", "test(policy): base")
    return repo


def commit_path(repo: Path, relative: str, data: bytes, message: str = "test(policy): object") -> str:
    target = repo / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    git_ok(repo, "add", "--", relative)
    git_ok(repo, "commit", "-q", "-m", message)
    return oid(repo)


def stage_path(repo: Path, relative: str, data: bytes, *, mode: int | None = None) -> None:
    target = repo / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    if mode is not None:
        target.chmod(mode)
    git_ok(repo, "add", "--", relative)


def stage_raw_path(repo: Path, relative: bytes, data: bytes, *, mode: int | None = None) -> None:
    """Stage a path whose Git bytes may not be valid UTF-8."""
    decoded = os.fsdecode(relative)
    target = repo / decoded
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    if mode is not None:
        target.chmod(mode)
    git_ok(repo, "add", "--", decoded)


def stage_forced_index_path(repo: Path, relative: bytes, data: bytes, *, mode: str = "100644") -> None:
    """Add arbitrary path bytes directly to the index without a filesystem path."""
    blob_oid = git_ok(repo, "hash-object", "-w", "--stdin", input_bytes=data).strip()
    require(re.fullmatch(rb"[0-9a-f]{40}", blob_oid) is not None, "Git did not return a SHA-1 blob OID for forced index fixture")
    cacheinfo = mode + "," + blob_oid.decode("ascii") + "," + os.fsdecode(relative)
    git_ok(repo, "update-index", "--add", "--cacheinfo", cacheinfo)


def staged_policy(repo: Path) -> subprocess.CompletedProcess[bytes]:
    return run([SCRIPTS / "check_staged_policy.sh", repo])


def approve(repo: Path, rationale: str = "benign regression fixture") -> subprocess.CompletedProcess[bytes]:
    return run([SCRIPTS / "approve_benign_staged_patterns.sh", repo, rationale])


def approval_path(repo: Path) -> Path:
    return Path(git_ok(repo, "rev-parse", "--absolute-git-dir").decode().strip()) / "whisperspree-staged-policy-approval"


def staged_added_manifest(repo: Path) -> dict[bytes, tuple[str, str]]:
    """Read Git's NUL-delimited raw index diff without passing paths through text."""
    raw = git_ok(repo, "diff", "--cached", "--raw", "-z", "--no-renames", "--no-abbrev")
    fields = raw.split(b"\0")
    manifest: dict[bytes, tuple[str, str]] = {}
    position = 0
    while position + 1 < len(fields) and fields[position]:
        header, path = fields[position], fields[position + 1]
        try:
            modes_and_oids, status = header[1:].rsplit(b" ", 1)
            old_mode, new_mode, old_oid, new_oid = modes_and_oids.split(b" ")
        except ValueError as error:
            raise RuntimeError("Git emitted an unexpected NUL raw-diff record") from error
        if status[:1] != b"D":
            manifest[path] = (new_mode.decode("ascii"), new_oid.decode("ascii"))
        position += 2
    return manifest


def parse_approval_manifest(record: bytes) -> dict[bytes, tuple[str, str]]:
    """Parse the test contract's NUL-safe approval artifact format.

    The remediation record is a single UTF-8 JSON object with an exact manifest
    array. Each path is base64 because approval must preserve arbitrary Git path
    bytes, including tabs and newlines, without line-oriented ambiguity.
    """
    try:
        document = json.loads(record.decode("utf-8"))
        entries = document["manifest"]
    except (UnicodeDecodeError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise AssertionError("approval artifact is not a valid NUL-safe manifest record") from error
    if document.get("version") != 1 or not isinstance(entries, list):
        raise AssertionError("approval artifact has no version-1 manifest")
    manifest: dict[bytes, tuple[str, str]] = {}
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"path_b64", "mode", "oid"}:
            raise AssertionError("approval artifact manifest entry is malformed")
        try:
            path = base64.b64decode(entry["path_b64"], validate=True)
        except (TypeError, ValueError) as error:
            raise AssertionError("approval artifact path is not base64") from error
        mode, blob_oid = entry["mode"], entry["oid"]
        if not isinstance(mode, str) or not isinstance(blob_oid, str) or len(blob_oid) != 40:
            raise AssertionError("approval artifact manifest entry lacks exact mode/blob OID")
        if path in manifest:
            raise AssertionError("approval artifact repeats a staged path")
        manifest[path] = (mode, blob_oid)
    return manifest


def sorted_raw_staged_manifest(repo: Path) -> list[dict[str, str]]:
    """Return the exact, NUL-safe raw index manifest expected by approval."""
    raw = git_ok(repo, "diff", "--cached", "--raw", "-z", "--no-renames", "--no-abbrev", "--no-ext-diff", "--no-textconv")
    fields = raw.split(b"\0")
    entries: list[dict[str, str]] = []
    position = 0
    while position + 1 < len(fields) and fields[position]:
        header, path = fields[position], fields[position + 1]
        try:
            modes_and_oids, status = header[1:].rsplit(b" ", 1)
            _old_mode, mode, _old_oid, blob_oid = modes_and_oids.split(b" ")
        except ValueError as error:
            raise RuntimeError("Git emitted an unexpected NUL raw-diff record") from error
        if status[:1] != b"D":
            entries.append({
                "path_b64": base64.b64encode(path).decode("ascii"),
                "mode": mode.decode("ascii"),
                "oid": blob_oid.decode("ascii"),
            })
        position += 2
    return sorted(entries, key=lambda entry: base64.b64decode(entry["path_b64"]))


def exact_raw_approval_document(record: Path) -> dict[str, object]:
    try:
        document = json.loads(record.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AssertionError("approval artifact is not a UTF-8 JSON document") from error
    if not isinstance(document, dict):
        raise AssertionError("approval artifact is not a JSON object")
    return document


def assert_rejected_without_marker(result: subprocess.CompletedProcess[bytes], explanation: str) -> bool:
    return result.returncode != 0 and MARKER not in result.stdout and MARKER not in result.stderr


def require_production_executable(relative: str) -> Path:
    candidate = ROOT / relative
    require(candidate.is_file() and os.access(candidate, os.X_OK), f"required production contract is missing or not executable: {relative}")
    return candidate


def copy_project_fixture(
    root: Path,
    name: str,
    *,
    branch: str,
    production_paths: tuple[str, ...],
) -> Path:
    """Build a disposable project with real relative wrapper layout and modes."""
    project = init_repo(root, name, branch)
    for relative in production_paths:
        source = require_production_executable(relative)
        destination = project / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        destination.chmod(stat.S_IMODE(source.stat().st_mode))
    shared_policy = SCRIPTS / "repo_policy.py"
    if shared_policy.is_file():
        destination = project / "scripts/repo_policy.py"
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(shared_policy, destination)
        destination.chmod(stat.S_IMODE(shared_policy.stat().st_mode))
    return project


def init_safe_push_project(root: Path, name: str, branch: str = "milestone0/t0-0-remediation") -> Path:
    project = copy_project_fixture(
        root,
        name,
        branch=branch,
        production_paths=("scripts/safe_git_push.sh", ".githooks/pre-push"),
    )
    git_ok(project, "remote", "add", "origin", CANONICAL_ORIGIN)
    return project


def init_safe_commit_project(root: Path, name: str) -> Path:
    return copy_project_fixture(
        root,
        name,
        branch="milestone0/t0-0-remediation",
        production_paths=(
            "scripts/safe_git_commit.sh",
            "scripts/check_staged_policy.sh",
            "scripts/check_attribution.sh",
            ".githooks/pre-commit",
            ".githooks/commit-msg",
        ),
    )


def safe_push(
    wrapper: Path,
    repo: Path,
    *args: str,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[bytes]:
    return run([wrapper, "--check-only", *args], cwd=repo, extra_env=env)


def safe_commit(
    repo: Path,
    message: str,
    *,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[bytes]:
    return run([repo / "scripts/safe_git_commit.sh", message], cwd=repo, extra_env=env)


def reset_canonical_origin(repo: Path) -> None:
    """Restore a single exact origin after each hostile configuration case."""
    git_ok(repo, "remote", "remove", "origin")
    git_ok(repo, "remote", "add", "origin", CANONICAL_ORIGIN)
    for key in (
        "core.sshCommand",
        "remote.origin.receivepack",
        "remote.origin.uploadpack",
        "remote.origin.mirror",
        f"branch.{git_ok(repo, 'branch', '--show-current').decode().strip()}.remote",
        f"branch.{git_ok(repo, 'branch', '--show-current').decode().strip()}.merge",
    ):
        result = git(repo, "config", "--unset-all", key)
        if result.returncode not in (0, 5):
            raise RuntimeError("could not restore canonical push-fixture configuration")
    for key in ("url.file:///tmp/unsafe.insteadOf", "url.file:///tmp/unsafe.pushInsteadOf"):
        result = git(repo, "config", "--unset-all", key)
        if result.returncode not in (0, 5):
            raise RuntimeError("could not restore canonical push-fixture URL configuration")


def execpolicy_response(*command_tokens: str, resolve_host_executables: bool = False) -> dict[str, object]:
    rules = ROOT / ".codex/rules/default.rules"
    require(rules.is_file(), "codex execpolicy rules file is missing")
    command: list[str | Path] = ["codex", "execpolicy", "check", "--rules", rules]
    if resolve_host_executables:
        command.append("--resolve-host-executables")
    command.extend(["--", *command_tokens])
    result = run(command)
    require(result.returncode == 0, "codex execpolicy check is unavailable or failed")
    try:
        response = json.loads(result.stdout.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AssertionError("codex execpolicy check returned no machine-readable decision") from error
    require(isinstance(response, dict), "codex execpolicy check returned an invalid response")
    decision = response.get("decision")
    require(decision is None or isinstance(decision, str), "codex execpolicy decision has an invalid shape")
    return response


def execpolicy_decision(*command_tokens: str, resolve_host_executables: bool = False) -> str | None:
    decision = execpolicy_response(*command_tokens, resolve_host_executables=resolve_host_executables).get("decision")
    return decision if isinstance(decision, str) else None


def assert_execpolicy_forbidden(*command_tokens: str) -> bool:
    return execpolicy_decision(*command_tokens) == "forbidden"


def require_all(checks: list[tuple[str, bool]]) -> None:
    failures = [label for label, passed in checks if not passed]
    require(not failures, "; ".join(failures))


def ci_secret_gate(repo: Path, event_name: str, event_file: Path, exceptions: Path | None = None) -> subprocess.CompletedProcess[bytes]:
    """Invoke the dedicated CI scanner; exceptions are explicit test input."""
    candidate = SCRIPTS / "check_ci_staged_policy.sh"
    if not candidate.is_file():
        return subprocess.CompletedProcess([str(candidate)], 127, b"", b"")
    command: list[str | Path] = [candidate, event_name, event_file, repo]
    if exceptions is not None:
        command.extend(["--exceptions", exceptions])
    return run(command)


def require_ci_secret_gate() -> None:
    candidate = SCRIPTS / "check_ci_staged_policy.sh"
    require(candidate.is_file() and os.access(candidate, os.X_OK), "dedicated CI staged-secret scanner is missing or not executable")


def staged_raw_blob_secret_detected_in_binary_nul_content(tmp: Path) -> None:
    repo = init_repo(tmp, "binary-nul")
    stage_path(repo, "payload.bin", b"\x00\xff" + MARKER + b"\x00")
    require(assert_rejected_without_marker(staged_policy(repo), "raw staged blob"), "raw NUL-containing staged blob was not rejected without leaking content")


def staged_scan_ignores_external_diff_and_textconv(tmp: Path) -> None:
    repo = init_repo(tmp, "diff-driver")
    stage_path(repo, "payload.bin", b"\x00" + MARKER + b"\x00")
    (repo / "omit-diff.sh").write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    (repo / "omit-diff.sh").chmod(0o700)
    git_ok(repo, "config", "diff.omit.command", str(repo / "omit-diff.sh"))
    git_ok(repo, "config", "diff.omit.textconv", str(repo / "omit-diff.sh"))
    (repo / ".gitattributes").write_text("payload.bin diff=omit\n", encoding="utf-8")
    git_ok(repo, "add", ".gitattributes")
    require(assert_rejected_without_marker(staged_policy(repo), "external diff"), "scanner trusted external diff or textconv output or leaked content")


def staged_scan_fails_closed_on_git_or_object_failure(tmp: Path) -> None:
    index_repo = init_repo(tmp, "missing-index")
    stage_path(index_repo, "payload.txt", MARKER)
    index = Path(git_ok(index_repo, "rev-parse", "--git-path", "index").decode().strip())
    if not index.is_absolute():
        index = index_repo / index
    index.unlink()
    index_failure = staged_policy(index_repo)

    object_repo = init_repo(tmp, "missing-blob")
    stage_path(object_repo, "payload.txt", MARKER)
    blob_oid = next(iter(staged_added_manifest(object_repo).values()))[1]
    object_path = Path(git_ok(object_repo, "rev-parse", "--absolute-git-dir").decode().strip()) / "objects" / blob_oid[:2] / blob_oid[2:]
    object_path.unlink()
    object_failure = staged_policy(object_repo)
    require_all([
        ("scanner accepted a Git/index command failure or leaked content", assert_rejected_without_marker(index_failure, "index")),
        ("scanner accepted an unavailable staged blob object or leaked content", assert_rejected_without_marker(object_failure, "object")),
    ])


def staged_manifest_handles_special_filenames_and_binds_path_mode_oid(tmp: Path) -> None:
    repo = init_repo(tmp, "manifest-special")
    special_paths = ["space name.txt", "tab\tname.txt", "-leading-dash.txt", "line\nbreak.txt"]
    for number, relative in enumerate(special_paths):
        stage_path(repo, relative, MARKER + bytes([number]), mode=0o755 if number == 0 else None)
    require(approve(repo).returncode == 0, "fixture approval could not be created")
    record = approval_path(repo).read_bytes()
    expected = staged_added_manifest(repo)
    actual = parse_approval_manifest(record)
    require(actual == expected, "approval manifest does not exactly bind NUL-safe path bytes, Git modes, and blob OIDs")


def approval_record_is_atomic_regular_0600_and_no_follow(tmp: Path) -> None:
    repo = init_repo(tmp, "approval-atomic")
    stage_path(repo, "notice.txt", MARKER)
    record = approval_path(repo)
    created_a = approve(repo)
    first_stat = record.lstat() if record.exists() else None
    first_bytes = record.read_bytes() if record.exists() else b""
    first_manifest_ok = False
    try:
        parse_approval_manifest(first_bytes)
        first_manifest_ok = True
    except AssertionError:
        pass
    old_descriptor = record.open("rb") if record.exists() else None
    stage_path(repo, "notice.txt", MARKER + b"-second")
    created_b = approve(repo)
    second_stat = record.lstat() if record.exists() else None
    second_bytes = record.read_bytes() if record.exists() else b""
    second_manifest_ok = False
    try:
        parse_approval_manifest(second_bytes)
        second_manifest_ok = True
    except AssertionError:
        pass
    old_descriptor_bytes = old_descriptor.read() if old_descriptor else b""
    if old_descriptor:
        old_descriptor.close()

    symlink_repo = init_repo(tmp, "approval-no-follow")
    stage_path(symlink_repo, "notice.txt", MARKER)
    symlink_record = approval_path(symlink_repo)
    sentinel = symlink_repo / "approval-sentinel"
    sentinel.write_bytes(b"sentinel bytes remain unchanged")
    symlink_record.symlink_to(sentinel)
    symlink_attempt = approve(symlink_repo)

    require_all([
        ("first approval was not created", created_a.returncode == 0),
        ("first approval is not a complete NUL-safe manifest", first_manifest_ok),
        ("first approval is not a regular 0600 file", first_stat is not None and stat.S_ISREG(first_stat.st_mode) and stat.S_IMODE(first_stat.st_mode) == 0o600),
        ("replacement approval was not created", created_b.returncode == 0),
        ("replacement approval is not a complete NUL-safe manifest", second_manifest_ok),
        ("replacement approval is not a regular 0600 file", second_stat is not None and stat.S_ISREG(second_stat.st_mode) and stat.S_IMODE(second_stat.st_mode) == 0o600),
        ("approval replacement reused the original inode", first_stat is not None and second_stat is not None and first_stat.st_ino != second_stat.st_ino),
        ("open descriptor did not retain complete original approval bytes", old_descriptor_bytes == first_bytes and bool(first_bytes)),
        ("approval creation followed a symlink target", symlink_attempt.returncode != 0 and sentinel.read_bytes() == b"sentinel bytes remain unchanged"),
    ])


def approval_record_rejects_symlink_insecure_malformed_and_changed_manifest(tmp: Path) -> None:
    def fresh(case: str) -> tuple[Path, Path, bool]:
        repo = init_repo(tmp, case)
        stage_path(repo, "notice.txt", MARKER)
        approved = approve(repo)
        baseline = staged_policy(repo)
        return repo, approval_path(repo), approved.returncode == 0 and baseline.returncode == 0

    symlink_repo, symlink_record, symlink_baseline = fresh("approval-symlink")
    sentinel = symlink_repo / "sentinel"
    valid_record_bytes = symlink_record.read_bytes()
    sentinel.write_bytes(valid_record_bytes)
    symlink_record.unlink()
    symlink_record.symlink_to(sentinel)
    symlink_result = staged_policy(symlink_repo)

    mode_repo, mode_record, mode_baseline = fresh("approval-mode")
    mode_record.chmod(0o644)
    insecure_mode_result = staged_policy(mode_repo)

    malformed_repo, malformed_record, malformed_baseline = fresh("approval-malformed")
    malformed_record.write_bytes(b"{")
    malformed_result = staged_policy(malformed_repo)

    content_repo, _, content_baseline = fresh("approval-content-drift")
    stage_path(content_repo, "notice.txt", MARKER + b"-changed")
    content_result = staged_policy(content_repo)

    git_mode_repo, _, git_mode_baseline = fresh("approval-git-mode-drift")
    stage_path(git_mode_repo, "notice.txt", MARKER, mode=0o755)
    git_mode_result = staged_policy(git_mode_repo)

    path_repo, _, path_baseline = fresh("approval-path-oid-drift")
    stage_path(path_repo, "renamed-notice.txt", MARKER)
    path_result = staged_policy(path_repo)

    require_all([
        ("fresh approval did not permit the exact benign staged marker", symlink_baseline),
        ("scanner followed a symlink approval record", assert_rejected_without_marker(symlink_result, "symlink") and sentinel.read_bytes() == valid_record_bytes),
        ("fresh approval did not permit the exact benign staged marker", mode_baseline),
        ("scanner accepted an insecure-mode approval", assert_rejected_without_marker(insecure_mode_result, "mode")),
        ("fresh approval did not permit the exact benign staged marker", malformed_baseline),
        ("scanner accepted a malformed or truncated approval", assert_rejected_without_marker(malformed_result, "malformed")),
        ("fresh approval did not permit the exact benign staged marker", content_baseline),
        ("scanner accepted staged content/blob drift", assert_rejected_without_marker(content_result, "content")),
        ("fresh approval did not permit the exact benign staged marker", git_mode_baseline),
        ("scanner accepted staged Git-mode drift", assert_rejected_without_marker(git_mode_result, "git mode")),
        ("fresh approval did not permit the exact benign staged marker", path_baseline),
        ("scanner accepted staged path/OID drift", assert_rejected_without_marker(path_result, "path")),
    ])


def safe_commit_wrapper_is_required_and_runs_both_checks(tmp: Path) -> None:
    wrapper = require_production_executable("scripts/safe_git_commit.sh")
    neutral_repo = init_safe_commit_project(tmp, "safe-commit-neutral")
    stage_path(neutral_repo, "normal.txt", b"normal")
    neutral_before = oid(neutral_repo)
    neutral = run([neutral_repo / "scripts/safe_git_commit.sh", "test(policy): neutral wrapper commit"], cwd=neutral_repo)
    neutral_committed = neutral.returncode == 0 and oid(neutral_repo) != neutral_before

    staged_repo = init_safe_commit_project(tmp, "safe-commit-staged-check")
    stage_path(staged_repo, "marker.txt", MARKER)
    staged_before = oid(staged_repo)
    staged = run([staged_repo / "scripts/safe_git_commit.sh", "test(policy): neutral metadata"], cwd=staged_repo)
    staged_rejected = assert_rejected_without_marker(staged, "staged") and oid(staged_repo) == staged_before

    attribution_repo = init_safe_commit_project(tmp, "safe-commit-attribution-check")
    stage_path(attribution_repo, "normal.txt", b"normal")
    attribution_before = oid(attribution_repo)
    attribution = run([attribution_repo / "scripts/safe_git_commit.sh", "test(policy): AI-assisted metadata"], cwd=attribution_repo)
    attribution_rejected = attribution.returncode != 0 and oid(attribution_repo) == attribution_before

    unrelated_repo = init_repo(tmp, "safe-commit-unrelated")
    stage_path(unrelated_repo, "normal.txt", b"normal")
    unrelated_before = oid(unrelated_repo)
    unrelated = run([wrapper, "test(policy): unrelated repository"], cwd=unrelated_repo)
    root_bound = unrelated.returncode != 0 and oid(unrelated_repo) == unrelated_before
    require_all([
        ("safe commit wrapper cannot commit neutral staged content with neutral metadata", neutral_committed),
        ("safe commit wrapper did not run staged-policy validation", staged_rejected),
        ("safe commit wrapper did not run attribution validation", attribution_rejected),
        ("root safe commit wrapper accepted an unrelated repository", root_bound),
    ])


def command_policy_forbids_commit_no_verify_short_n_and_direct_commit_producers(tmp: Path) -> None:
    del tmp
    commands = (
        ("git", "commit"),
        ("git", "commit", "--no-verify"),
        ("git", "commit", "-n"),
        ("/usr/bin/git", "commit"),
        ("command", "git", "commit"),
        ("env", "git", "commit"),
        ("git", "merge", "topic"),
        ("git", "cherry-pick", "HEAD"),
        ("git", "rebase", "main"),
        ("git", "am", "patch.mbox"),
        ("git", "commit-tree", "HEAD^{tree}"),
        ("git", "fast-import"),
    )
    require_all([
        ("codex execpolicy did not forbid a required commit-producing form", assert_execpolicy_forbidden(*command))
        for command in commands
    ])


def command_policy_forbids_absolute_git_keychain_and_common_wrapper_bypasses(tmp: Path) -> None:
    del tmp
    commands = (
        ("git", "push", "origin", "HEAD"),
        ("git", "push", "--force", "origin", "HEAD"),
        ("/usr/bin/git", "push", "origin", "HEAD"),
        ("command", "git", "push", "origin", "HEAD"),
        ("env", "git", "push", "origin", "HEAD"),
        ("/usr/bin/security", "find-generic-password", "-s", "whisperspree"),
        ("sh", "-c", "git commit -m x"),
        ("sh", "-c", "git push origin HEAD"),
        ("sh", "-c", "security find-generic-password -s whisperspree"),
        ("bash", "-c", "git commit -m x"),
        ("bash", "-c", "git push origin HEAD"),
        ("bash", "-c", "security find-generic-password -s whisperspree"),
        ("zsh", "-c", "git commit -m x"),
        ("zsh", "-c", "git push origin HEAD"),
        ("zsh", "-c", "security find-generic-password -s whisperspree"),
    )
    require_all([
        ("codex execpolicy did not forbid a required push/keychain wrapper form", assert_execpolicy_forbidden(*command))
        for command in commands
    ])


def safe_push_accepts_only_current_task_or_verification_branch(tmp: Path) -> None:
    task_repo = init_safe_push_project(tmp, "push-current")
    task_wrapper = task_repo / "scripts/safe_git_push.sh"
    task_accepted = safe_push(task_wrapper, task_repo, "origin", "HEAD:refs/heads/milestone0/t0-0-remediation").returncode == 0

    verification_repo = init_safe_push_project(tmp, "push-verification", "milestone0/verification")
    verification_wrapper = verification_repo / "scripts/safe_git_push.sh"
    verification_accepted = safe_push(verification_wrapper, verification_repo, "origin", "HEAD:refs/heads/milestone0/verification").returncode == 0
    invented_verification = safe_push(verification_wrapper, verification_repo, "origin", "HEAD:refs/heads/milestone0/t0-0-verification")
    near_miss_verification = safe_push(verification_wrapper, verification_repo, "origin", "HEAD:refs/heads/milestone0/verification-extra")
    wrong_task = safe_push(task_wrapper, task_repo, "origin", "HEAD:refs/heads/milestone0/t0-1-other")
    require_all([
        ("safe push rejected the current T0.0 task branch", task_accepted),
        ("safe push rejected the permitted T0.0 verification branch", verification_accepted),
        ("safe push accepted the obsolete t0-0 verification spelling", invented_verification.returncode != 0),
        ("safe push accepted a near-miss verification branch", near_miss_verification.returncode != 0),
        ("safe push accepted a different task branch", wrong_task.returncode != 0),
    ])


def safe_push_rejects_milestone_task_and_destination_mismatch(tmp: Path) -> None:
    repo = init_safe_push_project(tmp, "push-mismatch")
    wrapper = repo / "scripts/safe_git_push.sh"
    checks = [
        ("safe push accepted a destination different from the current branch", safe_push(wrapper, repo, "origin", "HEAD:refs/heads/milestone0/t0-0-different").returncode != 0),
        ("safe push accepted a mismatched task-major destination", safe_push(wrapper, repo, "origin", "HEAD:refs/heads/milestone0/t1-0-other").returncode != 0),
        ("safe push accepted a mismatched milestone destination", safe_push(wrapper, repo, "origin", "HEAD:refs/heads/milestone1/t1-0-other").returncode != 0),
    ]

    task_major_repo = init_safe_push_project(tmp, "push-task-major", "milestone0/t1-0-foreign")
    task_major_wrapper = task_major_repo / "scripts/safe_git_push.sh"
    checks.append((
        "safe push accepted a current branch whose task-major differs from its milestone",
        safe_push(task_major_wrapper, task_major_repo, "origin", "HEAD:refs/heads/milestone0/t1-0-foreign").returncode != 0,
    ))
    require_all(checks)


def safe_push_rejects_wrong_repo_detached_head_and_upstream_mismatch(tmp: Path) -> None:
    repo = init_safe_push_project(tmp, "push-identity")
    wrapper = repo / "scripts/safe_git_push.sh"
    git_ok(repo, "remote", "set-url", "origin", "https://github.com/other/whisperspree.git")
    wrong_repo_rejected = safe_push(wrapper, repo, "origin", "HEAD").returncode != 0
    reset_canonical_origin(repo)
    git_ok(repo, "checkout", "--detach", "HEAD")
    detached_rejected = safe_push(wrapper, repo, "origin", "HEAD").returncode != 0
    git_ok(repo, "checkout", "-q", "milestone0/t0-0-remediation")
    git_ok(repo, "config", "branch.milestone0/t0-0-remediation.remote", "origin")
    git_ok(repo, "config", "branch.milestone0/t0-0-remediation.merge", "refs/heads/milestone0/t0-1-other")
    upstream_rejected = safe_push(wrapper, repo, "origin", "HEAD").returncode != 0
    reset_canonical_origin(repo)

    unrelated_repo = init_repo(tmp, "root-wrapper-unrelated")
    git_ok(unrelated_repo, "remote", "add", "origin", CANONICAL_ORIGIN)
    root_wrapper_rejected = safe_push(require_production_executable("scripts/safe_git_push.sh"), unrelated_repo, "origin", "HEAD:refs/heads/milestone0/t0-0-remediation").returncode != 0

    worktree_parent = init_safe_push_project(tmp, "root-wrapper-worktree-parent")
    copied_fixture_paths = ["scripts/safe_git_push.sh", ".githooks/pre-push"]
    if (worktree_parent / "scripts/repo_policy.py").is_file():
        copied_fixture_paths.append("scripts/repo_policy.py")
    git_ok(worktree_parent, "add", *copied_fixture_paths)
    git_ok(worktree_parent, "commit", "-q", "-m", "test(policy): materialize safe push fixture")
    linked_worktree = tmp / "root-wrapper-linked-worktree"
    linked_branch = "milestone0/t0-0-linked"
    created_worktree = git(worktree_parent, "worktree", "add", "-b", linked_branch, linked_worktree, "HEAD")
    linked_worktree_rejected = True
    if created_worktree.returncode == 0:
        try:
            linked_worktree_rejected = safe_push(worktree_parent / "scripts/safe_git_push.sh", linked_worktree, "origin", f"HEAD:refs/heads/{linked_branch}").returncode != 0
        finally:
            removed_worktree = git(worktree_parent, "worktree", "remove", "--force", linked_worktree)
            if removed_worktree.returncode != 0:
                raise RuntimeError("could not remove disposable linked-worktree fixture")
    require_all([
        ("safe push accepted an untrusted origin", wrong_repo_rejected),
        ("safe push accepted detached HEAD", detached_rejected),
        ("safe push accepted an upstream mismatch", upstream_rejected),
        ("root safe push wrapper accepted an unrelated repository", root_wrapper_rejected),
        ("root safe push wrapper accepted a linked worktree", linked_worktree_rejected),
    ])


def safe_push_rejects_remapped_multiple_or_credentialed_origin(tmp: Path) -> None:
    repo = init_safe_push_project(tmp, "push-origin-forms")
    wrapper = repo / "scripts/safe_git_push.sh"
    checks: list[tuple[str, bool]] = []

    git_ok(repo, "remote", "set-url", "origin", "https://github.com/other/whisperspree.git")
    checks.append(("safe push accepted an untrusted origin", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)

    git_ok(repo, "remote", "set-url", "origin", "https://user:pass@github.com/Muminur/whisperspree.git")
    checks.append(("safe push accepted a credential-bearing canonical-host origin", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)

    git_ok(repo, "remote", "set-url", "--add", "origin", "https://github.com/Muminur/alternate.git")
    checks.append(("safe push accepted multiple origin URLs", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)

    git_ok(repo, "config", "--add", "remote.origin.pushurl", "https://github.com/Muminur/whisperspree.git")
    checks.append(("safe push accepted an origin pushurl", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)

    git_ok(repo, "config", "--add", "remote.origin.pushurl", "https://github.com/Muminur/whisperspree.git")
    git_ok(repo, "config", "--add", "remote.origin.pushurl", "https://github.com/Muminur/alternate.git")
    checks.append(("safe push accepted multiple origin pushurls", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)

    git_ok(repo, "config", "url.file:///tmp/unsafe.insteadOf", CANONICAL_ORIGIN)
    checks.append(("safe push accepted an insteadOf URL rewrite", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)

    git_ok(repo, "config", "url.file:///tmp/unsafe.pushInsteadOf", CANONICAL_ORIGIN)
    checks.append(("safe push accepted a pushInsteadOf URL rewrite", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)

    git_ok(repo, "remote", "set-url", "origin", "ext::custom-helper target")
    checks.append(("safe push accepted a custom remote-helper URL", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
    reset_canonical_origin(repo)
    require_all(checks)


def safe_push_rejects_receivepack_mirror_and_git_environment_injection(tmp: Path) -> None:
    repo = init_safe_push_project(tmp, "push-injection")
    wrapper = repo / "scripts/safe_git_push.sh"
    checks: list[tuple[str, bool]] = []
    checks.extend([
        ("safe push accepted a receive-pack command-line override", safe_push(wrapper, repo, "--receive-pack=custom-receive-pack", "origin", "HEAD").returncode != 0),
        ("safe push accepted a mirror command-line override", safe_push(wrapper, repo, "--mirror", "origin", "HEAD").returncode != 0),
    ])
    for key, value, label in (
        ("remote.origin.receivepack", "custom-receive-pack", "remote.origin.receivepack"),
        ("remote.origin.mirror", "true", "remote.origin.mirror"),
        ("core.sshCommand", "custom-ssh-command", "core.sshCommand"),
        ("remote.origin.uploadpack", "custom-upload-pack", "remote.origin.uploadpack"),
    ):
        git_ok(repo, "config", key, value)
        checks.append((f"safe push accepted {label}", safe_push(wrapper, repo, "origin", "HEAD").returncode != 0))
        reset_canonical_origin(repo)

    git_dir = Path(git_ok(repo, "rev-parse", "--absolute-git-dir").decode().strip())
    work_tree = Path(git_ok(repo, "rev-parse", "--show-toplevel").decode().strip())
    index_path = Path(git_ok(repo, "rev-parse", "--git-path", "index").decode().strip())
    if not index_path.is_absolute():
        index_path = (repo / index_path).resolve()
    for environment, label in (
        ({"GIT_DIR": str(git_dir)}, "GIT_DIR injection"),
        ({"GIT_WORK_TREE": str(work_tree)}, "GIT_WORK_TREE injection"),
        ({"GIT_INDEX_FILE": str(index_path)}, "GIT_INDEX_FILE injection"),
        ({"GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "remote.origin.receivepack", "GIT_CONFIG_VALUE_0": "git-receive-pack"}, "GIT_CONFIG_* injection"),
    ):
        checks.append((f"safe push accepted {label}", safe_push(wrapper, repo, "origin", "HEAD", env=environment).returncode != 0))
    require_all(checks)


def attribution_scanner_rejects_symlink_and_fails_closed_on_io_or_scanner_error(tmp: Path) -> None:
    clean = tmp / "clean-message"
    clean.write_text("test(policy): neutral\n", encoding="utf-8")
    link = tmp / "message-link"
    link.symlink_to(clean)
    symlink_rejected = run([SCRIPTS / "check_attribution.sh", link]).returncode != 0
    broken = tmp / "missing-message"
    unreadable_rejected = run([SCRIPTS / "check_attribution.sh", broken]).returncode != 0
    scanner_failure = run([SCRIPTS / "check_attribution.sh", clean], extra_env={"PATH": "/nonexistent"})
    require_all([
        ("attribution scanner followed a symlink", symlink_rejected),
        ("attribution scanner accepted unreadable metadata", unreadable_rejected),
        ("attribution scanner failed open when its scanner was unavailable", scanner_failure.returncode != 0),
    ])


def make_pr_event(path: Path, base: str, head: str) -> None:
    path.write_text(json.dumps({"pull_request": {"title": "neutral", "body": "", "base": {"sha": base}, "head": {"sha": head}}}), encoding="utf-8")


def make_push_event(path: Path, before: str, after: str, ref: str) -> None:
    path.write_text(json.dumps({"before": before, "after": after, "ref": ref}), encoding="utf-8")


def ci_secret_scan_pr_and_push_ranges_use_raw_blobs_and_ignore_diff_drivers(tmp: Path) -> None:
    repo = init_repo(tmp, "ci-pr-push")
    suppressor = repo / "suppress-diff.sh"
    suppressor.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    suppressor.chmod(0o700)
    git_ok(repo, "config", "diff.suppress.command", str(suppressor))
    git_ok(repo, "config", "diff.suppress.textconv", str(suppressor))
    (repo / ".gitattributes").write_text("binary.bin diff=suppress\n", encoding="utf-8")
    git_ok(repo, "add", ".gitattributes")
    git_ok(repo, "commit", "-q", "-m", "test(policy): configure suppressed diff driver")
    base = oid(repo)
    head = commit_path(repo, "binary.bin", b"\x00" + MARKER + b"\x00")
    pr = tmp / "pr.json"
    make_pr_event(pr, base, head)
    require_ci_secret_gate()
    push = tmp / "push.json"
    make_push_event(push, base, head, "refs/heads/milestone0/t0-0-remediation")
    pr_result = ci_secret_gate(repo, "pull_request", pr)
    push_result = ci_secret_gate(repo, "push", push)
    require_all([
        ("CI PR scan accepted a raw binary marker through external diff/textconv suppression", assert_rejected_without_marker(pr_result, "PR")),
        ("CI push scan accepted a raw binary marker through external diff/textconv suppression", assert_rejected_without_marker(push_result, "push")),
    ])


def ci_secret_scan_rejects_binary_marker_without_printing_content(tmp: Path) -> None:
    repo = init_repo(tmp, "ci-binary")
    base = oid(repo)
    head = commit_path(repo, "nul.bin", b"\x00" + MARKER + b"\x00")
    event = tmp / "event.json"
    make_pr_event(event, base, head)
    require_ci_secret_gate()
    result = ci_secret_gate(repo, "pull_request", event)
    require(result.returncode != 0, "CI scan accepted a binary marker")
    require(MARKER not in result.stdout and MARKER not in result.stderr, "CI scan printed matched marker content")


def ci_secret_scan_rejects_malformed_unavailable_nonancestor_and_nonfastforward_ranges(tmp: Path) -> None:
    repo = init_repo(tmp, "ci-ranges")
    base = oid(repo)
    head = commit_path(repo, "neutral.txt", b"neutral")
    malformed = tmp / "malformed.json"
    malformed.write_text("[]", encoding="utf-8")
    unavailable = tmp / "unavailable.json"
    make_pr_event(unavailable, base, "f" * 40)
    git_ok(repo, "checkout", "-q", "-b", "side", base)
    side_oid = commit_path(repo, "side.txt", b"side", "test(policy): side commit")
    git_ok(repo, "checkout", "-q", "milestone0/t0-0-remediation")
    nonancestor = tmp / "nonancestor.json"
    make_pr_event(nonancestor, side_oid, head)
    backward = tmp / "backward.json"
    make_push_event(backward, head, base, "refs/heads/milestone0/t0-0-remediation")
    require_ci_secret_gate()
    malformed_result = ci_secret_gate(repo, "pull_request", malformed)
    unavailable_result = ci_secret_gate(repo, "pull_request", unavailable)
    nonancestor_result = ci_secret_gate(repo, "pull_request", nonancestor)
    backward_result = ci_secret_gate(repo, "push", backward)
    require_all([
        ("CI scan accepted malformed event JSON or leaked content", assert_rejected_without_marker(malformed_result, "malformed range")),
        ("CI scan accepted unavailable range endpoint or leaked content", assert_rejected_without_marker(unavailable_result, "unavailable range")),
        ("CI scan accepted an available non-ancestor PR endpoint or leaked content", assert_rejected_without_marker(nonancestor_result, "non-ancestor range")),
        ("CI scan accepted a non-fast-forward push range or leaked content", assert_rejected_without_marker(backward_result, "non-fast-forward range")),
    ])


def ci_secret_scan_handles_zero_before_and_tag_targets_explicitly(tmp: Path) -> None:
    repo = init_repo(tmp, "ci-zero-tag")
    head = commit_path(repo, "neutral.txt", b"neutral")
    zero = tmp / "zero.json"
    make_push_event(zero, ZERO_OID, head, "refs/heads/milestone0/t0-0-remediation")
    git_ok(repo, "tag", "v0.0.0-lightweight", head)
    lightweight = tmp / "lightweight.json"
    make_push_event(lightweight, ZERO_OID, oid(repo, "v0.0.0-lightweight"), "refs/tags/v0.0.0-lightweight")
    git_ok(repo, "tag", "-a", "v0.0.0-annotated", "-m", "neutral tag", head)
    annotated = tmp / "annotated.json"
    annotated_oid = oid(repo, "v0.0.0-annotated")
    peeled_oid = oid(repo, "v0.0.0-annotated^{}")
    make_push_event(annotated, ZERO_OID, annotated_oid, "refs/tags/v0.0.0-annotated")
    require_ci_secret_gate()
    require_all([
        ("CI scan does not explicitly accept a valid zero-before range", ci_secret_gate(repo, "push", zero).returncode == 0),
        ("CI scan does not accept a lightweight tag target", ci_secret_gate(repo, "push", lightweight).returncode == 0),
        ("annotated tag did not produce a tag object distinct from its peeled commit", annotated_oid != peeled_oid),
        ("CI scan does not explicitly peel and accept an annotated tag target", ci_secret_gate(repo, "push", annotated).returncode == 0),
    ])


def tree_mode_and_oid(repo: Path, revision: str, relative: str) -> tuple[str, str]:
    listing = git_ok(repo, "ls-tree", "-z", revision, "--", relative)
    try:
        header, path = listing.rstrip(b"\0").split(b"\t", 1)
        mode, _kind, blob_oid = header.split(b" ")
    except ValueError as error:
        raise RuntimeError("Git tree fixture did not contain the requested blob") from error
    if path != relative.encode("utf-8"):
        raise RuntimeError("Git tree fixture returned the wrong path")
    return mode.decode("ascii"), blob_oid.decode("ascii")


def write_ci_exceptions(path: Path, entries: list[dict[str, str]]) -> None:
    """Write the scanner's explicit, exact v1 exception input contract."""
    path.write_text(json.dumps({"version": 1, "exceptions": entries}, sort_keys=True), encoding="utf-8")


def exact_ci_exception(relative: bytes, mode: str, blob_oid: str, rule: str = "token") -> dict[str, str]:
    return {
        "path_b64": base64.b64encode(relative).decode("ascii"),
        "mode": mode,
        "oid": blob_oid,
        "rule": rule,
    }


def ci_secret_exceptions_are_exact_path_mode_blob_rule_bound_and_drift_fails(tmp: Path) -> None:
    repo = init_repo(tmp, "ci-exceptions")
    base = oid(repo)
    head = commit_path(repo, "allowed-marker.txt", MARKER)
    event = tmp / "exception-event.json"
    make_pr_event(event, base, head)
    mode, marker_oid = tree_mode_and_oid(repo, head, "allowed-marker.txt")
    neutral_oid = tree_mode_and_oid(repo, base, "base.txt")[1]

    exact_file = tmp / "exact-exception.json"
    path_drift_file = tmp / "path-drift-exception.json"
    mode_drift_file = tmp / "mode-drift-exception.json"
    oid_drift_file = tmp / "oid-drift-exception.json"
    rule_drift_file = tmp / "rule-drift-exception.json"
    wildcard_file = tmp / "wildcard-exception.json"
    write_ci_exceptions(exact_file, [exact_ci_exception(b"allowed-marker.txt", mode, marker_oid)])
    write_ci_exceptions(path_drift_file, [exact_ci_exception(b"different-path.txt", mode, marker_oid)])
    write_ci_exceptions(mode_drift_file, [exact_ci_exception(b"allowed-marker.txt", "100755", marker_oid)])
    write_ci_exceptions(oid_drift_file, [exact_ci_exception(b"allowed-marker.txt", mode, neutral_oid)])
    write_ci_exceptions(rule_drift_file, [exact_ci_exception(b"allowed-marker.txt", mode, marker_oid, "different-rule")])
    wildcard_file.write_text(json.dumps({"version": 1, "exceptions": [{"path": "*", "mode": mode, "oid": marker_oid, "rule": "token"}]}), encoding="utf-8")

    require_ci_secret_gate()
    exact_result = ci_secret_gate(repo, "pull_request", event, exact_file)
    path_result = ci_secret_gate(repo, "pull_request", event, path_drift_file)
    mode_result = ci_secret_gate(repo, "pull_request", event, mode_drift_file)
    oid_result = ci_secret_gate(repo, "pull_request", event, oid_drift_file)
    rule_result = ci_secret_gate(repo, "pull_request", event, rule_drift_file)
    wildcard_result = ci_secret_gate(repo, "pull_request", event, wildcard_file)
    require_all([
        ("CI scanner rejected an exact path/mode/blob/rule exception", exact_result.returncode == 0),
        ("CI scanner accepted an exception after path drift", assert_rejected_without_marker(path_result, "path exception")),
        ("CI scanner accepted an exception after mode drift", assert_rejected_without_marker(mode_result, "mode exception")),
        ("CI scanner accepted an exception after blob/OID drift", assert_rejected_without_marker(oid_result, "oid exception")),
        ("CI scanner accepted an exception after rule drift", assert_rejected_without_marker(rule_result, "rule exception")),
        ("CI scanner accepted a broad/wildcard exception", assert_rejected_without_marker(wildcard_result, "wildcard exception")),
    ])


def workflow_has_separate_event_aware_secret_gate(tmp: Path) -> None:
    del tmp
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    require("Secret staged raw-blob policy" in workflow, "workflow has no separate event-aware staged-secret gate")
    require("check_ci_staged_policy.sh" in workflow and "GITHUB_EVENT_NAME" in workflow and "GITHUB_EVENT_PATH" in workflow, "workflow secret gate is not event-aware")


def policy_self_change_cannot_self_authorize_with_broad_exception(tmp: Path) -> None:
    repo = init_repo(tmp, "self-authorize")
    stage_path(repo, "scripts/check_staged_policy.sh", MARKER)
    require(approve(repo).returncode != 0, "policy self-change was eligible for benign-pattern approval")
    require(assert_rejected_without_marker(staged_policy(repo), "self authorization"), "policy self-change was allowed to self-authorize or leaked content")


def approval_record_has_exact_raw_only_schema_and_never_invokes_rendered_diff(tmp: Path) -> None:
    repo = init_repo(tmp, "approval-raw-only")
    stage_path(repo, "z-last.txt", MARKER)
    stage_path(repo, "a-first.txt", MARKER + b"-second", mode=0o755)
    shim_dir = tmp / "approval-git-shim"
    shim_dir.mkdir()
    shim_log = tmp / "approval-git-shim.log"
    real_git = shutil.which("git")
    require(real_git is not None, "Git executable is unavailable for the approval subprocess probe")
    shim = shim_dir / "git"
    shim.write_text(
        "#!/bin/sh\n"
        "printf '%s\\0' \"$@\" >>\"$GIT_SHIM_LOG\"\n"
        "printf '\\0' >>\"$GIT_SHIM_LOG\"\n"
        "is_diff=0\n"
        "has_raw=0\n"
        "for argument in \"$@\"; do\n"
        "  [ \"$argument\" = diff ] && is_diff=1\n"
        "  [ \"$argument\" = --raw ] && has_raw=1\n"
        "  case \"$argument\" in --binary|--textconv|--ext-diff) exit 97 ;; esac\n"
        "done\n"
        "[ \"$is_diff\" -eq 0 ] || [ \"$has_raw\" -eq 1 ] || exit 97\n"
        f"exec {real_git} \"$@\"\n",
        encoding="utf-8",
    )
    shim.chmod(0o700)
    approved = run(
        [SCRIPTS / "approve_benign_staged_patterns.sh", repo, "raw manifest approval"],
        extra_env={"PATH": str(shim_dir) + os.pathsep + os.environ.get("PATH", ""), "GIT_SHIM_LOG": str(shim_log)},
    )
    record = approval_path(repo)
    document = exact_raw_approval_document(record) if record.is_file() else {}
    manifest = document.get("manifest") if isinstance(document, dict) else None
    logged_invocations: list[list[bytes]] = []
    if shim_log.exists():
        current_invocation: list[bytes] = []
        for argument in shim_log.read_bytes().split(b"\0"):
            if argument:
                current_invocation.append(argument)
            elif current_invocation:
                logged_invocations.append(current_invocation)
                current_invocation = []
    require_all([
        ("approval could not be created without a rendered diff", approved.returncode == 0),
        ("approval record top-level schema is not exactly raw-only v1", set(document) == {"version", "approval_kind", "manifest", "rationale", "ruleset_sha256"}),
        ("approval record version is not v1", document.get("version") == 1),
        ("approval record does not identify the benign-pattern approval kind", document.get("approval_kind") == "benign-pattern"),
        ("approval record rationale is missing", document.get("rationale") == "raw manifest approval"),
        ("approval record ruleset digest is not an exact SHA-256", isinstance(document.get("ruleset_sha256"), str) and len(document["ruleset_sha256"]) == 64 and all(character in "0123456789abcdef" for character in document["ruleset_sha256"])),
        ("approval record manifest does not equal the exact sorted raw staged manifest", manifest == sorted_raw_staged_manifest(repo)),
        ("approval record manifest entry schema is not exact", isinstance(manifest, list) and all(isinstance(entry, dict) and set(entry) == {"path_b64", "mode", "oid"} for entry in manifest)),
        ("approval record retained a rendered-diff field", "staged_diff_sha256" not in document and all("diff" not in key for key in document)),
        ("approval subprocess invoked a rendered diff, textconv, or external diff", all(b"diff" not in invocation or (b"--raw" in invocation and not any(argument in {b"--binary", b"--textconv", b"--ext-diff"} for argument in invocation)) for invocation in logged_invocations)),
    ])


def explicit_policy_change_approval_is_exact_and_generic_mode_remains_blocked(tmp: Path) -> None:
    repo = init_repo(tmp, "explicit-policy-approval")
    rationale = "reviewed policy change with a benign detector marker"
    stage_path(repo, "scripts/check_staged_policy.sh", MARKER)
    expected_manifest = sorted_raw_staged_manifest(repo)
    generic = approve(repo, rationale)
    explicit = run([SCRIPTS / "approve_benign_staged_patterns.sh", "--policy-change", repo, rationale])
    record = approval_path(repo)
    document = exact_raw_approval_document(record) if record.is_file() else {}
    exact_staged_state = staged_policy(repo)

    stage_path(repo, "scripts/check_staged_policy.sh", MARKER + b"-content-drift")
    content_drift = staged_policy(repo)
    stage_path(repo, "scripts/check_staged_policy.sh", MARKER, mode=0o755)
    mode_drift = staged_policy(repo)
    stage_path(repo, "scripts/renamed-policy.sh", MARKER)
    path_drift = staged_policy(repo)

    governance_repo = init_repo(tmp, "explicit-policy-governance")
    stage_path(governance_repo, "AGENTS.md", MARKER)
    governance_generic = approve(governance_repo, rationale)
    governance_explicit = run([SCRIPTS / "approve_benign_staged_patterns.sh", "--policy-change", governance_repo, rationale])
    environment_repo = init_repo(tmp, "explicit-policy-environment")
    stage_path(environment_repo, ".env", MARKER)
    environment_generic = approve(environment_repo, rationale)
    environment_explicit = run([SCRIPTS / "approve_benign_staged_patterns.sh", "--policy-change", environment_repo, rationale])

    require_all([
        ("generic two-argument approval accepted a policy surface", generic.returncode != 0),
        ("explicit --policy-change approval interface did not create an approval", explicit.returncode == 0),
        ("explicit policy-change approval has the wrong raw-only schema, kind, ruleset binding, or frozen pre-drift manifest", set(document) == {"version", "approval_kind", "manifest", "rationale", "ruleset_sha256"} and document.get("version") == 1 and document.get("approval_kind") == "policy-change" and document.get("rationale") == rationale and isinstance(document.get("ruleset_sha256"), str) and len(document["ruleset_sha256"]) == 64 and document.get("manifest") == expected_manifest),
        ("explicit policy-change approval did not permit its unchanged exact staged state", exact_staged_state.returncode == 0),
        ("explicit policy-change approval survived content/OID drift", assert_rejected_without_marker(content_drift, "policy content drift")),
        ("explicit policy-change approval survived mode drift", assert_rejected_without_marker(mode_drift, "policy mode drift")),
        ("explicit policy-change approval survived path drift", assert_rejected_without_marker(path_drift, "policy path drift")),
        ("governance path was approvable in generic mode", governance_generic.returncode != 0),
        ("governance path was approvable in explicit policy-change mode", governance_explicit.returncode != 0),
        ("environment path was approvable in generic mode", environment_generic.returncode != 0),
        ("environment path was approvable in explicit policy-change mode", environment_explicit.returncode != 0),
    ])


def safe_commit_rejects_repository_index_config_and_tls_environment_injection(tmp: Path) -> None:
    checks: list[tuple[str, bool]] = []
    for number, (environment, label) in enumerate((
        ({"GIT_DIR": ".git"}, "GIT_DIR"),
        ({"GIT_WORK_TREE": "."}, "GIT_WORK_TREE"),
        ({"GIT_INDEX_FILE": ".git/index"}, "GIT_INDEX_FILE"),
        ({"GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.hooksPath", "GIT_CONFIG_VALUE_0": "/dev/null"}, "GIT_CONFIG_COUNT/KEY/VALUE"),
        ({"GIT_SSL_NO_VERIFY": "1"}, "GIT_SSL_NO_VERIFY"),
        ({"GIT_SSL_CAINFO": "/tmp/policy-test-ca.pem"}, "GIT_SSL_CAINFO"),
        ({"GIT_SSH_COMMAND": "/bin/false"}, "GIT_SSH_COMMAND"),
    )):
        repo = init_safe_commit_project(tmp, f"safe-commit-env-{number}")
        stage_path(repo, "normal.txt", b"normal")
        before = oid(repo)
        result = safe_commit(repo, "test(policy): hostile Git environment", env=environment)
        checks.append((f"safe commit accepted {label} injection or created a commit", result.returncode != 0 and oid(repo) == before))
    require_all(checks)


def safe_push_rejects_tls_and_transport_environment_injection(tmp: Path) -> None:
    repo = init_safe_push_project(tmp, "safe-push-tls")
    wrapper = repo / "scripts/safe_git_push.sh"
    checks = [
        (f"safe push accepted {label} injection", safe_push(wrapper, repo, "origin", "HEAD", env=environment).returncode != 0)
        for environment, label in (
            ({"GIT_SSL_NO_VERIFY": "1"}, "GIT_SSL_NO_VERIFY"),
            ({"GIT_SSL_CAINFO": "/tmp/policy-test-ca.pem"}, "GIT_SSL_CAINFO"),
            ({"GIT_SSH_COMMAND": "/bin/false"}, "GIT_SSH_COMMAND"),
        )
    ]
    require_all(checks)


def ci_exact_policy_path_exceptions_are_allowed_but_cannot_self_authorize(tmp: Path) -> None:
    repo = init_repo(tmp, "ci-policy-exceptions")
    base = oid(repo)
    stage_path(repo, "scripts/check_staged_policy.sh", MARKER)
    policy_oid = git_ok(repo, "hash-object", "scripts/check_staged_policy.sh").decode().strip()
    policy_mode = "100644"
    tracked_manifest = repo / ".repo-policy-secret-exceptions.json"
    write_ci_exceptions(tracked_manifest, [exact_ci_exception(b"scripts/check_staged_policy.sh", policy_mode, policy_oid)])
    git_ok(repo, "add", ".repo-policy-secret-exceptions.json")
    git_ok(repo, "commit", "-q", "-m", "test(policy): exact policy exception")
    approved_head = oid(repo)
    event = tmp / "policy-exception-event.json"
    make_pr_event(event, base, approved_head)
    exact = ci_secret_gate(repo, "pull_request", event, tracked_manifest)
    missing = ci_secret_gate(repo, "pull_request", event)

    path_drift = tmp / "policy-path-drift.json"
    mode_drift = tmp / "policy-mode-drift.json"
    oid_drift = tmp / "policy-oid-drift.json"
    rule_drift = tmp / "policy-rule-drift.json"
    wildcard = tmp / "policy-wildcard.json"
    extra = tmp / "policy-extra.json"
    malformed = tmp / "policy-malformed.json"
    write_ci_exceptions(path_drift, [exact_ci_exception(b"scripts/other.sh", policy_mode, policy_oid)])
    write_ci_exceptions(mode_drift, [exact_ci_exception(b"scripts/check_staged_policy.sh", "100755", policy_oid)])
    write_ci_exceptions(oid_drift, [exact_ci_exception(b"scripts/check_staged_policy.sh", policy_mode, tree_mode_and_oid(repo, approved_head, "base.txt")[1])])
    write_ci_exceptions(rule_drift, [exact_ci_exception(b"scripts/check_staged_policy.sh", policy_mode, policy_oid, "other")])
    wildcard.write_text(json.dumps({"version": 1, "exceptions": [{"path_b64": "*", "mode": policy_mode, "oid": policy_oid, "rule": "token"}]}), encoding="utf-8")
    extra.write_text(json.dumps({"version": 1, "exceptions": [exact_ci_exception(b"scripts/check_staged_policy.sh", policy_mode, policy_oid)], "arbitrary": True}), encoding="utf-8")
    malformed.write_text("{", encoding="utf-8")

    stage_path(repo, "scripts/check_staged_policy.sh", MARKER + b"-changed-policy")
    git_ok(repo, "commit", "-q", "-m", "test(policy): changed policy blob")
    changed_event = tmp / "changed-policy-event.json"
    make_pr_event(changed_event, approved_head, oid(repo))
    changed = ci_secret_gate(repo, "pull_request", changed_event, tracked_manifest)

    require_all([
        ("CI scanner rejected an exact policy-path exception", exact.returncode == 0),
        ("CI scanner accepted a missing policy exception", assert_rejected_without_marker(missing, "missing policy exception")),
        ("CI scanner accepted policy exception path drift", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", event, path_drift), "policy path drift")),
        ("CI scanner accepted policy exception mode drift", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", event, mode_drift), "policy mode drift")),
        ("CI scanner accepted policy exception OID drift", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", event, oid_drift), "policy OID drift")),
        ("CI scanner accepted policy exception rule drift", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", event, rule_drift), "policy rule drift")),
        ("CI scanner accepted a wildcard policy exception", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", event, wildcard), "policy wildcard")),
        ("CI scanner accepted a policy exception manifest with arbitrary extra data", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", event, extra), "policy extra field")),
        ("CI scanner accepted a malformed policy exception manifest", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", event, malformed), "policy malformed manifest")),
        ("CI scanner accepted a changed policy blob under a stale exception", assert_rejected_without_marker(changed, "changed policy blob")),
    ])


def workflow_binds_ci_secret_gate_to_exact_exception_manifest(tmp: Path) -> None:
    del tmp
    manifest = ROOT / ".repo-policy-secret-exceptions.json"
    tracked = run(["git", "ls-files", "--error-unmatch", ".repo-policy-secret-exceptions.json"])
    try:
        document = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        document = None
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    expected_command = 'scripts/check_ci_staged_policy.sh "${GITHUB_EVENT_NAME}" "${GITHUB_EVENT_PATH}" "${GITHUB_WORKSPACE}" --exceptions .repo-policy-secret-exceptions.json'
    secret_gate_runs: list[str] = []
    in_secret_gate_step = False
    for line in workflow.splitlines():
        if line.startswith("      - name: "):
            in_secret_gate_step = line == "      - name: Secret staged raw-blob policy"
        elif in_secret_gate_step and line.startswith("        run: "):
            secret_gate_runs.append(line.removeprefix("        run: "))
    entries = document.get("exceptions") if isinstance(document, dict) else None
    def is_strict_entry(entry: object) -> bool:
        if not isinstance(entry, dict) or set(entry) != {"path_b64", "mode", "oid", "rule"}:
            return False
        try:
            path = base64.b64decode(entry["path_b64"], validate=True)
        except (TypeError, ValueError):
            return False
        return (
            bool(path)
            and isinstance(entry["mode"], str)
            and re.fullmatch(r"[0-7]{6}", entry["mode"]) is not None
            and isinstance(entry["oid"], str)
            and re.fullmatch(r"[0-9a-f]{40}", entry["oid"]) is not None
            and entry["rule"] == "token"
        )

    strict_entries = isinstance(entries, list) and all(is_strict_entry(entry) for entry in entries)
    require_all([
        ("tracked narrow CI exception manifest is missing", tracked.returncode == 0 and manifest.is_file()),
        ("CI exception manifest is not strict v1 JSON", isinstance(document, dict) and set(document) == {"version", "exceptions"} and document.get("version") == 1 and strict_entries),
        ("Secret staged raw-blob policy workflow step is not bound to the exact exception manifest", secret_gate_runs == [expected_command]),
    ])


def ci_persistent_exact_exceptions_are_optional_on_later_neutral_ranges(tmp: Path) -> None:
    """A checked-in exact exception remains valid after its hit is no longer in range."""
    repo = init_repo(tmp, "optional-exception")
    base = oid(repo)
    exceptional = commit_path(repo, "grandfathered.bin", MARKER)
    neutral = commit_path(repo, "later-neutral.txt", b"later neutral change")
    mode, marker_oid = tree_mode_and_oid(repo, exceptional, "grandfathered.bin")
    exceptions = tmp / "persistent-exception.json"
    write_ci_exceptions(exceptions, [exact_ci_exception(b"grandfathered.bin", mode, marker_oid)])
    pr = tmp / "later-neutral-pr.json"
    push = tmp / "later-neutral-push.json"
    original = tmp / "original-exception-pr.json"
    make_pr_event(pr, exceptional, neutral)
    make_push_event(push, exceptional, neutral, "refs/heads/milestone0/t0-0-remediation")
    make_pr_event(original, base, exceptional)
    require_ci_secret_gate()
    require_all([
        ("an exact persistent exception blocked a later neutral PR range", ci_secret_gate(repo, "pull_request", pr, exceptions).returncode == 0),
        ("an exact persistent exception blocked a later neutral push range", ci_secret_gate(repo, "push", push, exceptions).returncode == 0),
        # The predecessor range proves this fixture is a real detector exception,
        # rather than an arbitrary unused manifest entry.
        ("the exact exception no longer permits its original matching blob", ci_secret_gate(repo, "pull_request", original, exceptions).returncode == 0),
    ])


def ci_scans_every_introduced_blob_across_pr_push_and_merge_history(tmp: Path) -> None:
    """Net tree diffs must not hide a blob introduced by an intermediate commit."""
    checks: list[tuple[str, bool]] = []

    add_delete = init_repo(tmp, "history-add-delete")
    add_delete_base = oid(add_delete)
    commit_path(add_delete, "removed.bin", b"\x00" + MARKER + b"\x00", "test(policy): add detector hit")
    add_delete_head = commit_path(add_delete, "removed.bin", b"neutral replacement", "test(policy): remove detector hit")
    add_delete_pr = tmp / "history-add-delete-pr.json"
    add_delete_push = tmp / "history-add-delete-push.json"
    make_pr_event(add_delete_pr, add_delete_base, add_delete_head)
    make_push_event(add_delete_push, add_delete_base, add_delete_head, "refs/heads/milestone0/t0-0-remediation")
    checks.extend([
        ("CI PR scan missed an add-then-delete detector hit", assert_rejected_without_marker(ci_secret_gate(add_delete, "pull_request", add_delete_pr), "add-delete PR")),
        ("CI push scan missed an add-then-delete detector hit", assert_rejected_without_marker(ci_secret_gate(add_delete, "push", add_delete_push), "add-delete push")),
    ])

    overwrite = init_repo(tmp, "history-overwrite")
    overwrite_base = oid(overwrite)
    commit_path(overwrite, "overwritten.bin", b"\x00" + MARKER + b"\x00", "test(policy): add detector hit")
    overwrite_head = commit_path(overwrite, "overwritten.bin", b"neutral overwrite", "test(policy): overwrite detector hit")
    overwrite_pr = tmp / "history-overwrite-pr.json"
    overwrite_push = tmp / "history-overwrite-push.json"
    make_pr_event(overwrite_pr, overwrite_base, overwrite_head)
    make_push_event(overwrite_push, overwrite_base, overwrite_head, "refs/heads/milestone0/t0-0-remediation")
    checks.extend([
        ("CI PR scan missed an add-then-neutral-overwrite detector hit", assert_rejected_without_marker(ci_secret_gate(overwrite, "pull_request", overwrite_pr), "overwrite PR")),
        ("CI push scan missed an add-then-neutral-overwrite detector hit", assert_rejected_without_marker(ci_secret_gate(overwrite, "push", overwrite_push), "overwrite push")),
    ])

    merged = init_repo(tmp, "history-merge")
    merge_base = oid(merged)
    git_ok(merged, "checkout", "-q", "-b", "feature", merge_base)
    commit_path(merged, "conflicted.bin", MARKER, "test(policy): feature detector hit")
    git_ok(merged, "checkout", "-q", "milestone0/t0-0-remediation")
    commit_path(merged, "conflicted.bin", b"base neutral content", "test(policy): base neutral content")
    git_ok(merged, "checkout", "-q", "feature")
    merge_attempt = git(merged, "merge", "--no-ff", "--no-commit", "milestone0/t0-0-remediation")
    require(merge_attempt.returncode != 0, "merge-history fixture did not produce its expected conflict")
    (merged / "conflicted.bin").write_bytes(b"merged neutral content")
    git_ok(merged, "add", "conflicted.bin")
    git_ok(merged, "commit", "-q", "-m", "test(policy): merge neutral resolution")
    merge_head = oid(merged)
    merge_pr = tmp / "history-merge-pr.json"
    merge_push = tmp / "history-merge-push.json"
    make_pr_event(merge_pr, merge_base, merge_head)
    make_push_event(merge_push, merge_base, merge_head, "refs/heads/feature")
    checks.extend([
        ("CI PR scan missed a detector hit in merge history", assert_rejected_without_marker(ci_secret_gate(merged, "pull_request", merge_pr), "merge PR")),
        ("CI push scan missed a detector hit in merge history", assert_rejected_without_marker(ci_secret_gate(merged, "push", merge_push), "merge push")),
    ])
    require_all(checks)


def ci_diverged_pr_scans_feature_side_commits_without_scanning_base_only_commits(tmp: Path) -> None:
    repo = init_repo(tmp, "diverged-pr")
    common = oid(repo)
    git_ok(repo, "checkout", "-q", "-b", "feature", common)
    feature_head = commit_path(repo, "feature-neutral.txt", b"feature-only neutral")
    git_ok(repo, "checkout", "-q", "milestone0/t0-0-remediation")
    base_head = commit_path(repo, "base-only.bin", MARKER, "test(policy): base-only detector marker")
    event = tmp / "diverged-pr.json"
    make_pr_event(event, base_head, feature_head)
    require_ci_secret_gate()
    result = ci_secret_gate(repo, "pull_request", event)
    require(result.returncode == 0, "CI scanner rejected a diverged PR whose feature side is neutral or scanned base-only commits")


def policy_surface_deletions_require_exact_explicit_approval_but_governance_untracking_is_allowed(tmp: Path) -> None:
    policy_paths = (
        "scripts/check_staged_policy.sh",
        ".githooks/pre-commit",
        ".github/workflows/policy.yml",
        ".codex/rules/default.rules",
        ".repo-policy-secret-exceptions.json",
    )
    repo = init_repo(tmp, "policy-deletions")
    expected_deletions: list[dict[str, str]] = []
    for relative in policy_paths:
        commit_path(repo, relative, b"tracked policy surface\n", "test(policy): track policy surface")
        mode, blob_oid = tree_mode_and_oid(repo, "HEAD", relative)
        expected_deletions.append({
            "path_b64": base64.b64encode(relative.encode("utf-8")).decode("ascii"),
            "old_mode": mode,
            "old_oid": blob_oid,
            "status": "D",
        })
        (repo / relative).unlink()
        git_ok(repo, "rm", "--", relative)
    generic = approve(repo, "generic deletion approval")
    explicit = run([SCRIPTS / "approve_benign_staged_patterns.sh", "--policy-change", repo, "reviewed policy deletion"])
    record = exact_raw_approval_document(approval_path(repo)) if approval_path(repo).is_file() else {}
    deletion_entries = record.get("deletions") if isinstance(record, dict) else None

    governance = init_repo(tmp, "governance-untracking")
    commit_path(governance, "AGENTS.md", b"bootstrap local governance\n", "test(policy): bootstrap governance")
    (governance / "AGENTS.md").unlink()
    git_ok(governance, "rm", "--", "AGENTS.md")
    require_all([
        ("generic approval accepted deletion of a policy surface", generic.returncode != 0),
        ("explicit policy approval did not accept every policy-surface deletion", explicit.returncode == 0),
        ("policy deletion approval did not bind deletion status and old path/mode/OID for every surface", deletion_entries == expected_deletions),
        ("exact explicit approval did not permit its unchanged policy-deletion index", staged_policy(repo).returncode == 0),
        ("bootstrap untracking of local-only governance was rejected", staged_policy(governance).returncode == 0),
    ])


def governance_and_environment_components_are_protected_locally_and_in_ci_with_raw_paths(tmp: Path) -> None:
    paths = (
        b"Agents.md",
        b"AGENTS.md",
        b"Loop.md",
        b"LOOP.md",
        b"nested/.env/file.txt",
        b"nested/.env.local/file.txt",
        b"nested/.env.\xff/file.txt",
    )
    checks: list[tuple[str, bool]] = []
    for number, raw_path in enumerate(paths):
        local = init_repo(tmp, f"protected-local-{number}")
        if b"\xff" in raw_path:
            stage_forced_index_path(local, raw_path, b"neutral protected-path fixture")
        else:
            stage_raw_path(local, raw_path, b"neutral protected-path fixture")
        checks.append((f"local staged policy accepted protected raw path {raw_path!r}", staged_policy(local).returncode != 0))

        ci = init_repo(tmp, f"protected-ci-{number}")
        base = oid(ci)
        if b"\xff" in raw_path:
            stage_forced_index_path(ci, raw_path, b"neutral protected-path fixture")
        else:
            stage_raw_path(ci, raw_path, b"neutral protected-path fixture")
        git_ok(ci, "commit", "-q", "-m", "test(policy): protected path")
        event = tmp / f"protected-ci-{number}.json"
        make_pr_event(event, base, oid(ci))
        checks.append((f"CI policy accepted protected raw path {raw_path!r}", ci_secret_gate(ci, "pull_request", event).returncode != 0))
    require_all(checks)


def json_inputs_reject_duplicate_keys_at_every_depth(tmp: Path) -> None:
    repo = init_repo(tmp, "duplicate-json")
    base = oid(repo)
    head = commit_path(repo, "marker.bin", MARKER)
    duplicate_event = tmp / "duplicate-event.json"
    duplicate_event.write_text(
        '{"pull_request":{"base":{"sha":"' + base + '","sha":"' + base + '"},"head":{"sha":"' + head + '"}}}',
        encoding="utf-8",
    )
    neutral = init_repo(tmp, "duplicate-trusted-manifest")
    neutral_base = oid(neutral)
    neutral_head = commit_path(neutral, "neutral.txt", b"neutral")
    normal_event = tmp / "normal-event.json"
    make_pr_event(normal_event, neutral_base, neutral_head)
    duplicate_top_level = tmp / "duplicate-top-level-trusted-manifest.json"
    duplicate_top_level.write_text(
        '{"version":2,"version":2,"secret_exceptions":[],"policy_transitions":[]}',
        encoding="utf-8",
    )
    duplicate_nested = tmp / "duplicate-nested-trusted-manifest.json"
    duplicate_nested.write_text(
        '{"version":2,"secret_exceptions":[],"policy_transitions":[{"path_b64":"c2NyaXB0cy9maXh0dXJlLnB5","status":"A","old_mode":"000000","old_oid":"' + ZERO_OID + '","new_mode":"100644","new_oid":"1111111111111111111111111111111111111111","new_oid":"1111111111111111111111111111111111111111"}]}',
        encoding="utf-8",
    )
    require_all([
        ("CI accepted duplicate event-object keys", ci_secret_gate(repo, "pull_request", duplicate_event).returncode != 0),
        ("trusted-base policy accepted duplicate top-level manifest keys", policy_ci(neutral, normal_event, base_manifest=duplicate_top_level).returncode != 0),
        ("trusted-base policy accepted duplicate nested manifest keys", policy_ci(neutral, normal_event, base_manifest=duplicate_nested).returncode != 0),
    ])


def policy_scanners_enforce_bounded_inputs_with_content_free_failures(tmp: Path) -> None:
    repo = init_repo(tmp, "bounded-inputs")
    base = oid(repo)
    head = commit_path(repo, "neutral.txt", b"neutral")
    oversized_event = tmp / "oversized-event.json"
    oversized_event.write_text(json.dumps({"pull_request": {"base": {"sha": base}, "head": {"sha": head}}, "padding": "x" * (1024 * 1024)}), encoding="utf-8")
    oversized_blob = init_repo(tmp, "oversized-blob")
    blob_base = oid(oversized_blob)
    blob_payload = b"q" * (2 * 1024 * 1024)
    blob_head = commit_path(oversized_blob, "large-neutral.bin", blob_payload)
    blob_event = tmp / "oversized-blob-event.json"
    make_pr_event(blob_event, blob_base, blob_head)

    exception_repo = init_repo(tmp, "too-many-exceptions")
    exception_base = oid(exception_repo)
    entries: list[dict[str, str]] = []
    # The policy contract deliberately caps exception fan-out at 16 entries and
    # commit walks at 8 commits; these limits keep CI work bounded while still
    # accommodating the project manifest's narrowly scoped historical entries.
    for number in range(17):
        relative = f"markers/{number}.bin"
        commit_path(exception_repo, relative, MARKER + str(number).encode("ascii"), "test(policy): detector fixture")
    exception_head = oid(exception_repo)
    for number in range(17):
        relative = f"markers/{number}.bin"
        mode, blob_oid = tree_mode_and_oid(exception_repo, exception_head, relative)
        entries.append(exact_ci_exception(relative.encode("ascii"), mode, blob_oid))
    too_many_exceptions = tmp / "too-many-exceptions.json"
    write_ci_exceptions(too_many_exceptions, entries)
    exception_event = tmp / "too-many-exceptions-event.json"
    make_pr_event(exception_event, exception_base, exception_head)

    commits_repo = init_repo(tmp, "too-many-commits")
    commits_base = oid(commits_repo)
    for number in range(9):
        commit_path(commits_repo, f"commits/{number}.txt", b"neutral", "test(policy): neutral history")
    commits_event = tmp / "too-many-commits-event.json"
    make_pr_event(commits_event, commits_base, oid(commits_repo))
    results = (
        ("oversized event JSON", ci_secret_gate(repo, "pull_request", oversized_event), b"x" * 32),
        ("oversized scanned blob", ci_secret_gate(oversized_blob, "pull_request", blob_event), b"q" * 32),
        ("too many exception entries", ci_secret_gate(exception_repo, "pull_request", exception_event, too_many_exceptions), MARKER),
        ("too many commits", ci_secret_gate(commits_repo, "pull_request", commits_event), b"neutral history"),
    )
    require_all([
        (f"scanner did not fail closed for {label} without printing content", result.returncode != 0 and forbidden not in result.stdout and forbidden not in result.stderr)
        for label, result, forbidden in results
    ])


def detector_rejects_representative_nonlive_secret_formats_and_entropy(tmp: Path) -> None:
    patterns = (
        b"AKIA" + b"A" * 16,
        # Assembled at runtime: a literal here would trip GitHub push
        # protection even though the token is entirely synthetic.
        b"xox" + b"b-000000000000-111111111111-abcdefghijklmnopqrstuvwxyzABCDEF",
        b"AIza" + b"A" * 35,
        b"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJub25saXZlIn0.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        b"https://fixture-user:fixture-pass@example.invalid/private.git",
        b"bXlOb25MaXZlRW50cm9weVJlZ3Jlc3Npb25WYWx1ZTEyMzQ1Njc4OTA=",
    )
    checks: list[tuple[str, bool]] = []
    for number, sample in enumerate(patterns):
        repo = init_repo(tmp, f"detector-format-{number}")
        stage_path(repo, f"sample-{number}.bin", b"\x00" + sample + b"\x00")
        result = staged_policy(repo)
        checks.append((f"detector accepted non-live secret format fixture {number}", result.returncode != 0 and sample not in result.stdout and sample not in result.stderr))
    require_all(checks)


def command_policy_rejects_path_loader_and_shell_startup_substitution(tmp: Path) -> None:
    del tmp
    commands = (
        ("env", "PATH=/tmp/policy-shim", "git", "push", "origin", "HEAD"),
        ("env", "PYTHONPATH=/tmp/policy-shim", "python3", "scripts/repo_policy.py", "staged", "."),
        ("env", "DYLD_INSERT_LIBRARIES=/tmp/policy-shim.dylib", "git", "commit"),
        ("env", "LD_PRELOAD=/tmp/policy-shim.so", "git", "commit"),
        ("env", "BASH_ENV=/tmp/policy-startup", "bash", "-c", "git push origin HEAD"),
        ("env", "ENV=/tmp/policy-startup", "sh", "-c", "git push origin HEAD"),
        ("env", "ZDOTDIR=/tmp/policy-startup", "zsh", "-c", "git push origin HEAD"),
    )
    require_all([
        ("command policy accepted executable/environment substitution: " + " ".join(command), assert_execpolicy_forbidden(*command))
        for command in commands
    ])


def safe_wrappers_reject_unsafe_git_config_but_allow_osxkeychain(tmp: Path) -> None:
    unsafe = (
        ("http.sslVerify", "false"),
        ("http.sslCAInfo", "/tmp/policy-ca.pem"),
        ("http.proxy", "http://proxy.invalid:8080"),
        ("http.extraHeader", "X-Policy-Test: true"),
        ("credential.helper", "store"),
        ("core.askPass", "/tmp/policy-askpass"),
        ("core.fsmonitor", "/tmp/policy-fsmonitor"),
        ("protocol.file.allow", "always"),
    )
    checks: list[tuple[str, bool]] = []
    push_repo = init_safe_push_project(tmp, "unsafe-git-config-push")
    commit_repo = init_safe_commit_project(tmp, "unsafe-git-config-commit")
    for number, (key, value) in enumerate(unsafe):
        git_ok(push_repo, "config", key, value)
        push_result = safe_push(push_repo / "scripts/safe_git_push.sh", push_repo, "origin", "HEAD")
        checks.append((f"safe push accepted unsafe Git configuration {key}", push_result.returncode != 0))
        git_ok(push_repo, "config", "--unset-all", key)

        git_ok(commit_repo, "config", key, value)
        stage_path(commit_repo, f"neutral-{number}.txt", b"neutral")
        commit_before = oid(commit_repo)
        commit_result = safe_commit(commit_repo, "test(policy): hostile Git configuration")
        checks.append((f"safe commit accepted unsafe Git configuration {key}", commit_result.returncode != 0 and oid(commit_repo) == commit_before))
        git_ok(commit_repo, "config", "--unset-all", key)
        git_ok(commit_repo, "reset", "--quiet")
    trusted = init_safe_push_project(tmp, "trusted-osxkeychain")
    git_ok(trusted, "config", "credential.helper", "osxkeychain")
    trusted_result = safe_push(trusted / "scripts/safe_git_push.sh", trusted, "origin", "HEAD")
    checks.append(("safe push rejected the trusted macOS osxkeychain credential helper", trusted_result.returncode == 0))
    require_all(checks)


def pre_push_scans_every_outgoing_ref_and_commit_class(tmp: Path) -> None:
    hook = require_production_executable(".githooks/pre-push")

    def invoke(repo: Path, records: list[tuple[str, str, str, str]]) -> subprocess.CompletedProcess[bytes]:
        payload = b"".join((" ".join(record) + "\n").encode("utf-8") for record in records)
        return run([hook, "origin", CANONICAL_ORIGIN], cwd=repo, input_bytes=payload)

    secret = init_repo(tmp, "pre-push-secret")
    secret_head = commit_path(secret, "secret.bin", MARKER)
    secret_result = invoke(secret, [("refs/heads/new-secret", secret_head, "refs/heads/new-secret", ZERO_OID)])

    attribution = init_repo(tmp, "pre-push-attribution")
    attribution_head = commit_path(attribution, "neutral.txt", b"neutral", "test(policy): AI-assisted metadata")
    attribution_result = invoke(attribution, [("refs/heads/new-attribution", attribution_head, "refs/heads/new-attribution", ZERO_OID)])

    policy = init_repo(tmp, "pre-push-policy")
    policy_head = commit_path(policy, "scripts/repo_policy.py", b"neutral scanner replacement")
    policy_result = invoke(policy, [("refs/heads/new-policy", policy_head, "refs/heads/new-policy", ZERO_OID)])

    tag = init_repo(tmp, "pre-push-tag")
    tag_commit = commit_path(tag, "tagged.bin", MARKER)
    git_ok(tag, "tag", "v-policy-test", tag_commit)
    tag_result = invoke(tag, [("refs/tags/v-policy-test", oid(tag, "v-policy-test"), "refs/tags/v-policy-test", ZERO_OID)])

    multi = init_repo(tmp, "pre-push-multi")
    neutral_head = commit_path(multi, "neutral.txt", b"neutral")
    marker_head = commit_path(multi, "marker.bin", MARKER)
    multi_result = invoke(multi, [
        ("refs/heads/neutral", neutral_head, "refs/heads/neutral", ZERO_OID),
        ("refs/heads/marker", marker_head, "refs/heads/marker", ZERO_OID),
    ])

    deletion = init_repo(tmp, "pre-push-deletion")
    deletion_head = oid(deletion)
    deletion_result = invoke(deletion, [("(delete)", ZERO_OID, "refs/heads/old", deletion_head)])

    nonfastforward = init_repo(tmp, "pre-push-nonfastforward")
    old_head = commit_path(nonfastforward, "old.txt", b"old")
    git_ok(nonfastforward, "checkout", "-q", "-b", "other", oid(nonfastforward, "HEAD~1"))
    other_head = commit_path(nonfastforward, "other.txt", b"other")
    nonfastforward_result = invoke(nonfastforward, [("refs/heads/other", other_head, "refs/heads/remote", old_head)])
    require_all([
        ("pre-push accepted a new ref containing a detector hit", assert_rejected_without_marker(secret_result, "pre-push secret")),
        ("pre-push accepted a new ref containing forbidden attribution", attribution_result.returncode != 0),
        ("pre-push accepted a policy/scanner change", policy_result.returncode != 0),
        ("pre-push accepted a tag target containing a detector hit", assert_rejected_without_marker(tag_result, "pre-push tag")),
        ("pre-push accepted a detector hit in a later multi-ref input record", assert_rejected_without_marker(multi_result, "pre-push multi-ref")),
        ("pre-push accepted a deletion", deletion_result.returncode != 0),
        ("pre-push accepted a non-fast-forward update", nonfastforward_result.returncode != 0),
    ])


def workflow_has_base_trusted_policy_job_without_pr_head_execution(tmp: Path) -> None:
    del tmp
    workflow_dir = ROOT / ".github/workflows"
    ordinary = (workflow_dir / "ci.yml").read_text(encoding="utf-8")
    candidates = [path for path in workflow_dir.glob("*.yml") if path.name != "ci.yml"] + [path for path in workflow_dir.glob("*.yaml")]
    policy_workflows = [path for path in candidates if "pull_request_target" in path.read_text(encoding="utf-8")]
    require("pull_request:" in ordinary and "pnpm test" in ordinary and "cargo test" in ordinary, "ordinary pull_request build/test workflow is missing")
    require(len(policy_workflows) == 1, "exactly one separate base-trusted pull_request_target policy workflow is required")
    policy = policy_workflows[0].read_text(encoding="utf-8")
    require("contents: read" in policy and "persist-credentials: false" in policy, "base-trusted policy workflow is not read-only")
    require("actions/checkout" in policy and "github.event.pull_request.base.sha" in policy, "base-trusted policy workflow does not explicitly checkout the base revision")
    require("check_ci_staged_policy.sh" in policy and "check_ci_attribution.sh" in policy, "base-trusted policy workflow does not run both policy gates")
    require("github.event.pull_request.head.sha" not in policy and "github.sha" not in policy, "base-trusted policy workflow can checkout or execute PR-head code")
    require("pnpm " not in policy and "cargo " not in policy and "npm " not in policy, "base-trusted policy workflow executes PR build/test commands")


def command_policy_rejects_global_shell_wrapper_gh_and_http_mutation_bypasses(tmp: Path) -> None:
    del tmp
    commands = (
        ("git", "--git-dir=/tmp/policy.git", "push", "origin", "HEAD"),
        ("git", "--work-tree", "/tmp", "commit"),
        ("git", "--no-pager", "credential", "fill"),
        ("sh", "-lc", "  git   push origin HEAD"),
        ("bash", "--noprofile", "-c", "\tgit commit -m x"),
        ("zsh", "-f", "-c", " git push origin HEAD "),
        ("/opt/homebrew/bin/git", "push", "origin", "HEAD"),
        ("env", "PATH=/tmp", "git", "commit"),
        ("python3", "-c", "import subprocess; subprocess.run(['git', 'push', 'origin', 'HEAD'])"),
        ("gh", "pr", "create"),
        ("gh", "pr", "merge", "1"),
        ("gh", "repo", "edit", "--visibility", "private"),
        ("gh", "api", "--method", "POST", "/repos/Muminur/whisperspree/issues"),
        ("curl", "-X", "POST", "https://api.github.com/repos/Muminur/whisperspree/issues"),
        ("http", "POST", "https://api.github.com/repos/Muminur/whisperspree/issues"),
    )
    require_all([
        ("command policy accepted mutation/credential bypass: " + " ".join(command), assert_execpolicy_forbidden(*command))
        for command in commands
    ])


def self_issued_generic_approval_cannot_authorize_deletion_scanner_manifest_or_unstaged_hook(tmp: Path) -> None:
    policy_targets = (
        "scripts/repo_policy.py",
        ".repo-policy-secret-exceptions.json",
    )
    checks: list[tuple[str, bool]] = []
    for number, relative in enumerate(policy_targets):
        repo = init_repo(tmp, f"self-issued-{number}")
        commit_path(repo, relative, b"tracked policy target\n", "test(policy): track policy target")
        (repo / relative).unlink()
        git_ok(repo, "rm", "--", relative)
        checks.append((f"generic approval authorized self-issued deletion of {relative}", approve(repo, "self-issued approval").returncode != 0 and staged_policy(repo).returncode != 0))

    hook_repo = init_safe_commit_project(tmp, "unstaged-hook-replacement")
    stage_path(hook_repo, "neutral.txt", b"neutral")
    (hook_repo / ".githooks/pre-commit").write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    replaced_hook = safe_commit(hook_repo, "test(policy): neutral staged content")
    checks.append(("safe commit accepted an unstaged pre-commit-hook replacement", replaced_hook.returncode != 0))
    require_all(checks)


# T0.0 remediation contracts.  These replace the legacy tests that treated
# Codex's literal prefix rules and same-user approval files as a semantic
# security boundary.  Every dynamic case below uses real local Git objects;
# fixtures deliberately contain only non-live detector markers.
FIXED_CODEX_FILES = {
    ".codex/config.toml": "d49608d7f1ddeed9f5754d63ce10908436cc5b11dab4126eb822cc171022d713",
    ".codex/agents/architect.toml": "ab2e7ab63dc721a222a80d9f80cd23ed881205178a5f4da88922ce1d7a1f56ca",
    ".codex/agents/branch_reviewer.toml": "93160b41761150d19a60db99a0d9e96c0533903b76fb86fc90031f69dc3935f7",
    ".codex/agents/ci_guard.toml": "4fd3974f71c37f22ebe6a9e2ee4c9a414e43f45a4da8fecb0efdea3c7571f29d",
    ".codex/agents/code_reviewer.toml": "aba27ce51fb54b9d4188ffba8f24054982b720586ecefa3770ddc6b61d425ac6",
    ".codex/agents/explorer.toml": "d136e29ebd20ae7ba78d15a78a4e2807d0d262678311da1f29737644635c8b92",
    ".codex/agents/implementer.toml": "b9324666070691fa0bde420e8d2748b1c618d68173c41dcb90fc274d97a9bac3",
    ".codex/agents/merger.toml": "1570ea4f5a98189eaea697c3dd84505995f50b9b4309c262751a0ff29baacc8f",
    ".codex/agents/milestone_auditor.toml": "6b975361eae0839a31d28505330ed1d5b55fdbf8f25cc328a68f49dda1c61dce",
    ".codex/agents/security_reviewer.toml": "2ec9e8607f2a2ab462350044273dcae033a554e3d8b86dbfb75f49a88a86b82d",
    ".codex/agents/tdd_author.toml": "d3298fbdab1032aae646dacbadc02b986f11c180dadc8e122f874295efe06b17",
}


def policy_ci(root: Path, event: Path, *, base_manifest: Path | None = None) -> subprocess.CompletedProcess[bytes]:
    """The new CI interface must receive authorization from a separate base tree."""
    command: list[str | Path] = [SCRIPTS / "repo_policy.py", "ci", "pull_request", event, root]
    if base_manifest is not None:
        command.extend(["--trusted-manifest", base_manifest])
    return run(command)


def transition_entry(path: bytes, status: str, old_mode: str, old_oid: str, new_mode: str, new_oid: str) -> dict[str, str]:
    return {
        "path_b64": base64.b64encode(path).decode("ascii"), "status": status,
        "old_mode": old_mode, "old_oid": old_oid, "new_mode": new_mode, "new_oid": new_oid,
    }


def write_trusted_manifest(path: Path, transitions: list[dict[str, str]], exceptions: list[dict[str, str]] | None = None) -> None:
    path.write_text(json.dumps({"version": 2, "secret_exceptions": exceptions or [], "policy_transitions": transitions}, sort_keys=True), encoding="utf-8")


def execpolicy_is_literal_three_force_push_denials_without_broad_allows(tmp: Path) -> None:
    del tmp
    rules = (ROOT / ".codex/rules/default.rules").read_text(encoding="utf-8")
    force_forms = (("git", "push", "--force", "origin", "HEAD"), ("git", "push", "--force-with-lease", "origin", "HEAD"), ("git", "push", "-f", "origin", "HEAD"))
    normal_forms = (("git", "status"), ("git", "add", "README.md"), ("git", "commit", "-m", "test"), ("git", "push", "origin", "HEAD"), ("gh", "pr", "create"), ("gh", "pr", "checks"), ("gh", "pr", "merge", "1"))
    absolute_git = "/usr/bin/git"
    checks = [(f"force form was not forbidden: {' '.join(form)}", execpolicy_decision(*form) == "forbidden") for form in force_forms]
    checks.extend((f"ordinary operation was categorically forbidden: {' '.join(form)}", execpolicy_decision(*form) != "forbidden") for form in normal_forms)
    checks.extend((f"approved absolute force form was not forbidden: {' '.join(form)}", execpolicy_decision(absolute_git, *form[1:], resolve_host_executables=True) == "forbidden") for form in force_forms)
    checks.extend([
        ("rules contain an allow decision", "decision = \"allow\"" not in rules),
        ("rules still claim shell wrappers are semantically covered", execpolicy_decision("sh", "-c", "git push --force origin HEAD") != "forbidden"),
        ("rules lack audited host-executable Git confinement", "host_executable" in rules and "name = \"git\"" in rules),
    ])
    require_all(checks)


def codex_fixed_config_and_ten_roles_are_byte_identical(tmp: Path) -> None:
    del tmp
    require_all([(f"fixed Codex baseline drifted: {relative}", (ROOT / relative).is_file() and hashlib.sha256((ROOT / relative).read_bytes()).hexdigest() == digest) for relative, digest in FIXED_CODEX_FILES.items()])


def policy_plumbing_uses_audited_git_and_rejects_path_loader_and_scoped_config_controls(tmp: Path) -> None:
    repo = init_safe_push_project(tmp, "audited-plumbing")
    shim = tmp / "hostile-bin"; shim.mkdir()
    sentinel = tmp / "shim-ran"
    for executable in ("git", "python3"):
        target = shim / executable
        target.write_text(f"#!/bin/sh\nprintf shim > {sentinel}\nexit 97\n", encoding="utf-8")
        target.chmod(0o700)
    baseline = safe_push(repo / "scripts/safe_git_push.sh", repo, "origin", "HEAD", env={"PATH": str(shim) + os.pathsep + "/usr/bin:/bin"})
    controls = (("include.path", "/tmp/policy-include"), ("url.https://wrong.invalid/.insteadOf", CANONICAL_ORIGIN), ("remote.origin.receivepack", "/tmp/receive-pack"), ("core.hooksPath", "/tmp/hooks"), ("diff.driver.command", "/tmp/diff"), ("http.sslVerify", "false"), ("http.proxy", "http://proxy.invalid"), ("http.extraHeader", "Authorization: fixture"), ("credential.helper", "store"), ("core.fsmonitor", "/tmp/fsmonitor"))
    results: list[tuple[str, bool]] = [("policy plumbing inherited hostile PATH or executed its shim", baseline.returncode != 97 and not sentinel.exists())]
    for key, value in controls:
        git_ok(repo, "config", key, value)
        result = safe_push(repo / "scripts/safe_git_push.sh", repo, "origin", "HEAD")
        results.append((f"policy plumbing accepted controlling scoped configuration {key}", result.returncode != 0))
        git_ok(repo, "config", "--unset-all", key)
    # On macOS, SIP can strip DYLD_INSERT_LIBRARIES before /bin/sh starts.
    # That is a safe result: no policy process can inherit the control.  Keep
    # the hostile PATH shim in this probe so a pass is accepted only when no
    # shim ran; a surviving control must still be rejected by the wrapper.
    dyld = safe_push(
        repo / "scripts/safe_git_push.sh",
        repo,
        "origin",
        "HEAD",
        env={
            "DYLD_INSERT_LIBRARIES": "/tmp/policy-control.dylib",
            "PATH": str(shim) + os.pathsep + "/usr/bin:/bin",
        },
    )
    results.append((
        "policy plumbing executed its hostile shim or handled DYLD_INSERT_LIBRARIES unexpectedly",
        dyld.returncode in {0, 2} and not sentinel.exists(),
    ))
    for variable in ("LD_PRELOAD", "PYTHONPATH", "BASH_ENV", "ENV", "ZDOTDIR"):
        results.append((f"policy plumbing accepted loader/startup control {variable}", safe_push(repo / "scripts/safe_git_push.sh", repo, "origin", "HEAD", env={variable: "/tmp/policy-control"}).returncode != 0))
    git_ok(repo, "config", "credential.helper", "osxkeychain")
    results.append(("policy plumbing rejected exact sanctioned osxkeychain helper", safe_push(repo / "scripts/safe_git_push.sh", repo, "origin", "HEAD").returncode == 0))
    require_all(results)


def mutable_local_wrapper_is_advisory_and_never_the_pr_policy_authority(tmp: Path) -> None:
    base = init_repo(tmp, "trusted-base", branch="main")
    fork = init_repo(tmp, "untrusted-fork", branch="feature")
    sentinel = fork / "head-executed"; commit_path(fork, "scripts/repo_policy.py", b"import pathlib; pathlib.Path('head-executed').write_text('executed')\n")
    git_ok(fork, "config", "include.path", "/tmp/hostile-config")
    event = tmp / "fork-event.json"; event.write_text(json.dumps({"repository": {"full_name": "Muminur/whisperspree"}, "pull_request": {"number": 7, "base": {"sha": oid(base)}, "head": {"sha": oid(fork), "repo": {"clone_url": str(fork)}}}}), encoding="utf-8")
    workflow = (ROOT / ".github/workflows/pr-policy.yml").read_text(encoding="utf-8")
    result = policy_ci(base, event)
    require_all([
        ("base-trusted workflow uses a PR-head executable path", "github.event.pull_request.head.sha" not in workflow and "allow-unsafe-pr-checkout" not in workflow),
        ("base policy scanner did not reject the untrusted raw-object detector hit", result.returncode != 0),
        ("base policy workflow executed mutable fork wrapper/scanner code", not sentinel.exists()),
        ("Codex policy granted a broad mutable-wrapper allow", "decision = \"allow\"" not in (ROOT / ".codex/rules/default.rules").read_text(encoding="utf-8")),
        ("base policy engine lacks the required isolated raw-object fetch interface", "fetch_pr_objects" in (SCRIPTS / "repo_policy.py").read_text(encoding="utf-8")),
    ])


def self_issued_approval_artifacts_cannot_authorize_any_finding(tmp: Path) -> None:
    repo = init_repo(tmp, "self-issued-artifacts")
    stage_path(repo, "ordinary-secret.bin", MARKER)
    issued = approve(repo, "self-issued benign marker")
    valid_artifact = staged_policy(repo)
    approval = approval_path(repo); approval.write_text(json.dumps({"version": 1, "approval_kind": "benign-pattern", "manifest": [], "rationale": "self-issued", "ruleset_sha256": "0" * 64}), encoding="utf-8")
    malformed = staged_policy(repo)
    approval.unlink(); approval.symlink_to(tmp / "elsewhere")
    symlink = staged_policy(repo)
    policy_repo = init_repo(tmp, "self-issued-policy")
    commit_path(policy_repo, "scripts/repo_policy.py", b"base scanner")
    (policy_repo / "scripts/repo_policy.py").unlink(); git_ok(policy_repo, "rm", "scripts/repo_policy.py")
    legacy = approve(policy_repo, "self-issued policy deletion")
    require_all([
        ("a valid self-issued approval artifact authorized a detector finding", issued.returncode != 0 and assert_rejected_without_marker(valid_artifact, "valid legacy approval")),
        ("legacy approval artifact authorized a detector finding", assert_rejected_without_marker(malformed, "legacy approval")),
        ("symlinked legacy approval artifact authorized a detector finding", assert_rejected_without_marker(symlink, "symlinked approval")),
        ("legacy approval command remained able to authorize a policy deletion", legacy.returncode != 0 and staged_policy(policy_repo).returncode != 0),
    ])


def exact_exception_never_skips_unrelated_nonpolicy_scan(tmp: Path) -> None:
    repo = init_repo(tmp, "exception-no-early-return")
    base = oid(repo)
    head = commit_path(repo, "scripts/approved.py", b"approved policy replacement")
    head = commit_path(repo, "ordinary-secret.bin", MARKER)
    event = tmp / "event.json"; make_pr_event(event, base, head)
    old_mode, old_oid = tree_mode_and_oid(repo, base, "base.txt")
    new_mode, new_oid = tree_mode_and_oid(repo, head, "scripts/approved.py")
    manifest = tmp / "trusted.json"; write_trusted_manifest(manifest, [transition_entry(b"scripts/approved.py", "A", "000000", ZERO_OID, new_mode, new_oid)])
    result = policy_ci(repo, event, base_manifest=manifest)
    policy_only = init_repo(tmp, "exception-policy-only"); policy_only_base = oid(policy_only); policy_only_head = commit_path(policy_only, "scripts/approved.py", b"approved policy replacement")
    policy_only_event = tmp / "policy-only.json"; make_pr_event(policy_only_event, policy_only_base, policy_only_head)
    mode, blob_oid = tree_mode_and_oid(policy_only, policy_only_head, "scripts/approved.py")
    policy_only_manifest = tmp / "policy-only-trusted.json"; write_trusted_manifest(policy_only_manifest, [transition_entry(b"scripts/approved.py", "A", "000000", ZERO_OID, mode, blob_oid)])
    require_all([
        ("trusted exact exception did not permit its policy entry", policy_ci(policy_only, policy_only_event, base_manifest=policy_only_manifest).returncode == 0),
        ("trusted policy exception skipped unrelated secret scan", assert_rejected_without_marker(result, "unrelated secret")),
    ])


def trusted_base_two_pr_policy_transition_allows_exact_add_modify_delete_and_replacement(tmp: Path) -> None:
    checks: list[tuple[str, bool]] = []
    for number, (status, initial, target) in enumerate((("A", None, b"added"), ("M", b"old", b"modified"), ("D", b"removed", None), ("A", None, b"replacement"))):
        repo = init_repo(tmp, f"transition-{number}"); base = oid(repo); path = "scripts/target.py" if number != 3 else ".githooks/replacement"
        if initial is not None: commit_path(repo, path, initial)
        transition_base = oid(repo)
        if target is None:
            (repo / path).unlink(); git_ok(repo, "rm", path); git_ok(repo, "commit", "-q", "-m", "test(policy): delete target")
        else:
            target_path = repo / path; target_path.parent.mkdir(parents=True, exist_ok=True); target_path.write_bytes(target); git_ok(repo, "add", path); git_ok(repo, "commit", "-q", "-m", "test(policy): transition target")
        head = oid(repo); event = tmp / f"transition-{number}.json"; make_pr_event(event, transition_base, head)
        old_mode, old_oid = ("000000", ZERO_OID) if initial is None else tree_mode_and_oid(repo, transition_base, path)
        new_mode, new_oid = ("000000", ZERO_OID) if target is None else tree_mode_and_oid(repo, head, path)
        trusted = tmp / f"trusted-{number}.json"; write_trusted_manifest(trusted, [transition_entry(path.encode(), status, old_mode, old_oid, new_mode, new_oid)])
        exact = policy_ci(repo, event, base_manifest=trusted)
        drift = tmp / f"drift-{number}.json"
        if status == "D":
            # A deletion has a required zero new OID, so changing new_oid would
            # reproduce the exact tuple. Drift the bound old OID instead.
            write_trusted_manifest(drift, [transition_entry(path.encode(), status, old_mode, ZERO_OID, new_mode, new_oid)])
        else:
            write_trusted_manifest(drift, [transition_entry(path.encode(), status, old_mode, old_oid, new_mode, ZERO_OID)])
        checks.extend([(f"trusted-base exact {status} transition did not pass for {path}", exact.returncode == 0), (f"trusted-base exact transition accepted a bound tuple drift for {path}", policy_ci(repo, event, base_manifest=drift).returncode != 0)])
    require_all(checks)


def head_manifest_cannot_self_authorize_or_mix_manifest_and_policy_change(tmp: Path) -> None:
    repo = init_repo(tmp, "head-manifest")
    base = oid(repo); stage_path(repo, "scripts/repo_policy.py", b"head policy change"); stage_path(repo, ".repo-policy-secret-exceptions.json", b'{"version":2,"secret_exceptions":[],"policy_transitions":[]}'); git_ok(repo, "commit", "-q", "-m", "test(policy): self authorize")
    head = oid(repo); event = tmp / "head.json"; make_pr_event(event, base, head)
    base_manifest = tmp / "base.json"; write_trusted_manifest(base_manifest, [])
    self_authorized = policy_ci(repo, event, base_manifest=repo / ".repo-policy-secret-exceptions.json")
    mixed = policy_ci(repo, event, base_manifest=base_manifest)
    require_all([
        ("head manifest authorized its own policy transition", self_authorized.returncode != 0),
        ("manifest update mixed with policy change was accepted", mixed.returncode != 0),
        ("policy CI did not expose a trusted-base manifest input", "--trusted-manifest" in (SCRIPTS / "repo_policy.py").read_text(encoding="utf-8")),
    ])


def policy_streams_long_pr_branch_tag_and_exception_sets_without_truncation(tmp: Path) -> None:
    repo = init_repo(tmp, "long-history", branch="main"); base = oid(repo)
    for number in range(10): commit_path(repo, f"history/{number}.txt", b"neutral", "test(policy): neutral history")
    head = oid(repo); pr = tmp / "long-pr.json"; make_pr_event(pr, base, head)
    branch = tmp / "long-branch.json"; make_push_event(branch, ZERO_OID, head, "refs/heads/milestone0/t0-0-remediation")
    git_ok(repo, "tag", "-a", "v-long", "-m", "neutral tag", head); tag = tmp / "long-tag.json"; make_push_event(tag, ZERO_OID, oid(repo, "v-long"), "refs/tags/v-long")
    exceptions = tmp / "many-exceptions.json"; write_ci_exceptions(exceptions, [exact_ci_exception(f"unused/{number}".encode(), "100644", tree_mode_and_oid(repo, head, "base.txt")[1]) for number in range(17)])
    late = commit_path(repo, "history/late.bin", b"\0" + MARKER + b"\0"); late_event = tmp / "late.json"; make_pr_event(late_event, base, late)
    require_all([
        ("long neutral PR history was truncated or rejected", ci_secret_gate(repo, "pull_request", pr).returncode == 0),
        ("long new-branch history was truncated or rejected", ci_secret_gate(repo, "push", branch).returncode == 0),
        ("long annotated-tag history was truncated or rejected", ci_secret_gate(repo, "push", tag).returncode == 0),
        ("valid manifest with more than sixteen exact entries was rejected", ci_secret_gate(repo, "pull_request", pr, exceptions).returncode == 0),
        ("late detector hit after eight commits was missed", assert_rejected_without_marker(ci_secret_gate(repo, "pull_request", late_event), "late history hit")),
    ])


def config_agents_and_gitignore_are_policy_surfaces_locally_outgoing_and_in_ci(tmp: Path) -> None:
    surfaces = (".codex/config.toml", ".codex/agents/special name.toml", ".gitignore")
    checks: list[tuple[str, bool]] = []
    hook = require_production_executable(".githooks/pre-push")
    for number, path in enumerate(surfaces):
        repo = init_repo(tmp, f"missing-surface-{number}"); base = oid(repo); stage_path(repo, path, b"policy surface")
        local = staged_policy(repo); git_ok(repo, "commit", "-q", "-m", "test(policy): policy surface")
        head = oid(repo); event = tmp / f"surface-{number}.json"; make_pr_event(event, base, head)
        pre_push = run([hook, "origin", CANONICAL_ORIGIN], cwd=repo, input_bytes=f"refs/heads/test {head} refs/heads/test {ZERO_OID}\n".encode())
        checks.extend([(f"staged policy failed to classify {path} as a transition surface", local.returncode != 0), (f"outgoing policy failed to classify {path} as a transition surface", pre_push.returncode != 0), (f"CI policy failed to classify {path} as a transition surface", ci_secret_gate(repo, "pull_request", event).returncode != 0)])
    require_all(checks)


def pre_push_rejects_governance_in_every_outgoing_ref_and_intermediate_commit(tmp: Path) -> None:
    hook = require_production_executable(".githooks/pre-push")
    paths = ("Agents.md", "Loop.md", "docs/PRD.md", "nested/.env.fixture")
    checks: list[tuple[str, bool]] = []
    for number, path in enumerate(paths):
        repo = init_repo(tmp, f"outgoing-governance-{number}"); start = oid(repo); commit_path(repo, path, b"governance leak")
        (repo / path).unlink(); git_ok(repo, "rm", path); git_ok(repo, "commit", "-q", "-m", "test(policy): remove leaked governance")
        head = oid(repo); payload = f"refs/heads/one {head} refs/heads/one {start}\nrefs/heads/two {head} refs/heads/two {ZERO_OID}\n".encode()
        result = run([hook, "origin", CANONICAL_ORIGIN], cwd=repo, input_bytes=payload)
        checks.append((f"pre-push accepted outgoing/intermediate governance path {path}", result.returncode != 0))
    require_all(checks)


def detector_rejects_quoted_json_assignment_and_bearer_authorization_forms(tmp: Path) -> None:
    forms = (b'{"token": "quoted-nonlive-value-abcdef"}', b'"api_key" = "quoted-nonlive-value-abcdef"', b'Authorization: Bearer nonlive-authorization-marker-abcdef')
    checks: list[tuple[str, bool]] = []
    for number, form in enumerate(forms):
        repo = init_repo(tmp, f"quoted-secret-{number}"); base = oid(repo); stage_path(repo, f"binary/{number}.bin", b"\0" + form + b"\0")
        staged = staged_policy(repo); git_ok(repo, "commit", "-q", "-m", "test(policy): quoted detector fixture"); event = tmp / f"quoted-{number}.json"; make_pr_event(event, base, oid(repo)); ci = ci_secret_gate(repo, "pull_request", event)
        checks.extend([(f"staged detector accepted quoted/Bearer form {number}", staged.returncode != 0 and form not in staged.stdout and form not in staged.stderr), (f"CI detector accepted quoted/Bearer form {number}", ci.returncode != 0 and form not in ci.stdout and form not in ci.stderr)])
    require_all(checks)


def pull_request_target_fetches_validated_fork_objects_without_checkout_or_execution(tmp: Path) -> None:
    base = init_repo(tmp, "fetch-base", branch="main"); bare = tmp / "fork.git"; must_run(["git", "init", "--bare", "-q", bare])
    fork = init_repo(tmp, "fetch-fork", branch="feature"); sentinel = fork / "executed"; commit_path(fork, "scripts/never-run.py", b"import pathlib; pathlib.Path('executed').write_text('bad')"); head = commit_path(fork, "late.bin", b"\0" + MARKER + b"\0")
    git_ok(fork, "remote", "add", "origin", str(bare)); git_ok(fork, "push", "-q", "origin", f"HEAD:refs/pull/42/head")
    base_head = oid(base); event = tmp / "valid-fork.json"; event.write_text(json.dumps({"repository": {"full_name": "Muminur/whisperspree"}, "pull_request": {"number": 42, "base": {"sha": base_head}, "head": {"sha": head, "repo": {"clone_url": str(bare)}}}}), encoding="utf-8")
    result = policy_ci(base, event)
    require_all([( "valid fork raw objects were not fetched and scanned", assert_rejected_without_marker(result, "fork marker")), ("fork object fetch checked out or executed head content", not sentinel.exists() and oid(base) == base_head), ("workflow lacks isolated raw-object fetch contract", "fetch_pr_objects" in (SCRIPTS / "repo_policy.py").read_text(encoding="utf-8"))])


def fork_fetch_rejects_ref_sha_repo_event_and_object_mismatch_content_free(tmp: Path) -> None:
    base = init_repo(tmp, "mismatch-base", branch="main"); event_base = oid(base); cases = []
    payloads = (
        "{\"repository\": {\"full_name\": \"wrong/repo\"}, \"pull_request\": {}}",
        "{\"repository\": {\"full_name\": \"Muminur/whisperspree\", \"full_name\": \"duplicate\"}}",
        json.dumps({"repository": {"full_name": "Muminur/whisperspree"}, "pull_request": {"number": 1, "base": {"sha": "f" * 40}, "head": {"sha": "e" * 40}}}),
    )
    for number, payload in enumerate(payloads):
        event = tmp / f"mismatch-{number}.json"; event.write_text(payload, encoding="utf-8"); result = policy_ci(base, event); cases.append((f"fork fetch accepted mismatch {number} or leaked payload", result.returncode != 0 and payload.encode() not in result.stdout and payload.encode() not in result.stderr))
    require_all(cases + [
        ("mismatch fixture unexpectedly changed trusted base", oid(base) == event_base),
        ("fork mismatch handling is not backed by the validated raw-object fetch interface", "fetch_pr_objects" in (SCRIPTS / "repo_policy.py").read_text(encoding="utf-8")),
    ])


def pr_policy_workflow_is_base_only_and_declares_post_merge_bootstrap_gate(tmp: Path) -> None:
    del tmp
    policy = (ROOT / ".github/workflows/pr-policy.yml").read_text(encoding="utf-8"); plan = (ROOT / "docs/plans/T0.0.md").read_text(encoding="utf-8")
    require_all([
        ("PR-policy workflow is not pull_request_target-only", "pull_request_target" in policy and "pull_request:" not in policy),
        ("PR-policy workflow does not pin base checkout with read-only credentials", "github.event.pull_request.base.sha" in policy and "persist-credentials: false" in policy and "contents: read" in policy),
        ("PR-policy workflow can checkout or execute head code", all(token not in policy for token in ("github.event.pull_request.head.sha", "github.sha", "allow-unsafe-pr-checkout", "pnpm ", "npm ", "cargo ", "source "))),
        ("bootstrap protocol lacks post-merge canary and required-check condition", "neutral fork PR" in plan and "required branch-protection" in plan),
        ("PR-policy workflow lacks validated raw-object acquisition", "fetch_pr_objects" in (SCRIPTS / "repo_policy.py").read_text(encoding="utf-8")),
    ])


def every_remote_workflow_action_is_full_sha_pinned_and_provenanced(tmp: Path) -> None:
    del tmp
    ledger = (ROOT / "docs/DEPENDENCIES.md").read_text(encoding="utf-8"); checks: list[tuple[str, bool]] = []
    for workflow in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
        for action in re.findall(r"^\s*uses:\s*([^\s#]+)", workflow.read_text(encoding="utf-8"), flags=re.M):
            owner_repo, separator, revision = action.partition("@")
            checks.append((f"workflow action is not a full lowercase SHA pin: {workflow.name}:{action}", bool(separator) and re.fullmatch(r"[0-9a-f]{40}", revision) is not None and owner_repo in ledger and revision in ledger and "2026-08-05" in ledger))
    require(checks, "no remote workflow actions were found to audit")
    require_all(checks)


TESTS: list[tuple[str, Callable[[Path], None]]] = [
    ("staged_raw_blob_secret_detected_in_binary_nul_content", staged_raw_blob_secret_detected_in_binary_nul_content),
    ("staged_scan_ignores_external_diff_and_textconv", staged_scan_ignores_external_diff_and_textconv),
    ("staged_scan_fails_closed_on_git_or_object_failure", staged_scan_fails_closed_on_git_or_object_failure),
    ("safe_push_accepts_only_current_task_or_verification_branch", safe_push_accepts_only_current_task_or_verification_branch),
    ("safe_push_rejects_milestone_task_and_destination_mismatch", safe_push_rejects_milestone_task_and_destination_mismatch),
    ("safe_push_rejects_wrong_repo_detached_head_and_upstream_mismatch", safe_push_rejects_wrong_repo_detached_head_and_upstream_mismatch),
    ("safe_push_rejects_remapped_multiple_or_credentialed_origin", safe_push_rejects_remapped_multiple_or_credentialed_origin),
    ("safe_push_rejects_receivepack_mirror_and_git_environment_injection", safe_push_rejects_receivepack_mirror_and_git_environment_injection),
    ("attribution_scanner_rejects_symlink_and_fails_closed_on_io_or_scanner_error", attribution_scanner_rejects_symlink_and_fails_closed_on_io_or_scanner_error),
    ("ci_secret_scan_pr_and_push_ranges_use_raw_blobs_and_ignore_diff_drivers", ci_secret_scan_pr_and_push_ranges_use_raw_blobs_and_ignore_diff_drivers),
    ("ci_secret_scan_rejects_binary_marker_without_printing_content", ci_secret_scan_rejects_binary_marker_without_printing_content),
    ("ci_secret_scan_rejects_malformed_unavailable_nonancestor_and_nonfastforward_ranges", ci_secret_scan_rejects_malformed_unavailable_nonancestor_and_nonfastforward_ranges),
    ("ci_secret_scan_handles_zero_before_and_tag_targets_explicitly", ci_secret_scan_handles_zero_before_and_tag_targets_explicitly),
    ("ci_scans_every_introduced_blob_across_pr_push_and_merge_history", ci_scans_every_introduced_blob_across_pr_push_and_merge_history),
    ("ci_diverged_pr_scans_feature_side_commits_without_scanning_base_only_commits", ci_diverged_pr_scans_feature_side_commits_without_scanning_base_only_commits),
    ("governance_and_environment_components_are_protected_locally_and_in_ci_with_raw_paths", governance_and_environment_components_are_protected_locally_and_in_ci_with_raw_paths),
    ("json_inputs_reject_duplicate_keys_at_every_depth", json_inputs_reject_duplicate_keys_at_every_depth),
    ("detector_rejects_representative_nonlive_secret_formats_and_entropy", detector_rejects_representative_nonlive_secret_formats_and_entropy),
    ("execpolicy_is_literal_three_force_push_denials_without_broad_allows", execpolicy_is_literal_three_force_push_denials_without_broad_allows),
    ("codex_fixed_config_and_ten_roles_are_byte_identical", codex_fixed_config_and_ten_roles_are_byte_identical),
    ("policy_plumbing_uses_audited_git_and_rejects_path_loader_and_scoped_config_controls", policy_plumbing_uses_audited_git_and_rejects_path_loader_and_scoped_config_controls),
    ("mutable_local_wrapper_is_advisory_and_never_the_pr_policy_authority", mutable_local_wrapper_is_advisory_and_never_the_pr_policy_authority),
    ("self_issued_approval_artifacts_cannot_authorize_any_finding", self_issued_approval_artifacts_cannot_authorize_any_finding),
    ("exact_exception_never_skips_unrelated_nonpolicy_scan", exact_exception_never_skips_unrelated_nonpolicy_scan),
    ("trusted_base_two_pr_policy_transition_allows_exact_add_modify_delete_and_replacement", trusted_base_two_pr_policy_transition_allows_exact_add_modify_delete_and_replacement),
    ("head_manifest_cannot_self_authorize_or_mix_manifest_and_policy_change", head_manifest_cannot_self_authorize_or_mix_manifest_and_policy_change),
    ("policy_streams_long_pr_branch_tag_and_exception_sets_without_truncation", policy_streams_long_pr_branch_tag_and_exception_sets_without_truncation),
    ("config_agents_and_gitignore_are_policy_surfaces_locally_outgoing_and_in_ci", config_agents_and_gitignore_are_policy_surfaces_locally_outgoing_and_in_ci),
    ("pre_push_rejects_governance_in_every_outgoing_ref_and_intermediate_commit", pre_push_rejects_governance_in_every_outgoing_ref_and_intermediate_commit),
    ("detector_rejects_quoted_json_assignment_and_bearer_authorization_forms", detector_rejects_quoted_json_assignment_and_bearer_authorization_forms),
    ("pull_request_target_fetches_validated_fork_objects_without_checkout_or_execution", pull_request_target_fetches_validated_fork_objects_without_checkout_or_execution),
    ("fork_fetch_rejects_ref_sha_repo_event_and_object_mismatch_content_free", fork_fetch_rejects_ref_sha_repo_event_and_object_mismatch_content_free),
    ("pr_policy_workflow_is_base_only_and_declares_post_merge_bootstrap_gate", pr_policy_workflow_is_base_only_and_declares_post_merge_bootstrap_gate),
    ("every_remote_workflow_action_is_full_sha_pinned_and_provenanced", every_remote_workflow_action_is_full_sha_pinned_and_provenanced),
]


def main(selected_names: list[str]) -> int:
    by_name = dict(TESTS)
    unknown = [name for name in selected_names if name not in by_name]
    if unknown:
        print("unknown test selection: " + ", ".join(unknown), file=sys.stderr)
        return 2
    selected = [(name, by_name[name]) for name in selected_names] if selected_names else TESTS
    passed = 0
    failed: list[tuple[str, str]] = []
    with tempfile.TemporaryDirectory(prefix="whisperspree-t0-0-policy-") as tempdir:
        temporary_root = Path(tempdir)
        for name, test in selected:
            case_dir = temporary_root / name
            case_dir.mkdir()
            try:
                test(case_dir)
            except (AssertionError, RuntimeError, OSError, subprocess.SubprocessError) as error:
                failed.append((name, str(error).replace(MARKER.decode(), "[redacted-marker]")))
                print(f"FAIL {name}: {failed[-1][1]}")
            else:
                passed += 1
                print(f"PASS {name}")
    print(f"SUMMARY total={len(selected)} passed={passed} failed={len(failed)}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
