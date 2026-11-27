"""Promotion guard of the vanilla differential oracle (owner D128, review D131).

A fix must not make itself pass by deleting, disabling or ignoring tests, and must not
change what the recorded corpus replays. `ledger promote` calls `promotion_guard` for
every core fix (a commit or a `patch:` fix) on the exact tree it promotes (HEAD plus the
uncommitted state), before any control runs:

- the baseline is that same tree with the whole fix reversed, test hunks included. A fix
  that no longer reverses there as a whole is refused (re-record it as a `patch:` of the
  candidate), so hunks outside a patch's hashed identity cannot offset anything and no
  historical tree stands in for the promoted one;
- tests are compared by identity: `cargo nextest list` of every package the fix touches,
  on both trees, or of the whole workspace when the fix touches a `Cargo.toml` or any
  path outside `crates/` (`.config/nextest.toml`, `Cargo.lock`, `.cargo/`, data a build
  reads). A test the baseline compiles and runs that the promoted tree no longer lists,
  or lists as ignored or filtered out, refuses the promotion, so a dropped `mod`
  declaration, a deleted test replaced by another one, a new `#[ignore]` or a nextest
  filter are all caught; a moved test changes identity and is refused too (land the move
  as its own commit); assertion counts are not compared;
- `corpus-lock.jsonl`: a removed line, or a new line for a case the baseline already has
  (a new active version), refuses it; new cases may be added. A re-record is its own
  commit with a reason, never part of a fix.

Changes outside the fix (later commits, uncommitted edits the fix does not contain) are
in both trees and are not the fix's doing; test-prune commits are separate, listed and
reviewed commits.
"""
from __future__ import annotations

import json
import subprocess
import tomllib
from collections.abc import Callable
from pathlib import Path

LOCK_REL = "tools/harness/quirk_parity/corpus-lock.jsonl"
EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"

# (tree, packages or None for the whole workspace) -> {"<binary-id>::<test name>": not run}
Lister = Callable[[Path, list[str] | None], dict[str, bool]]


class GuardRefused(RuntimeError):
    pass


def _git(tree: Path, *args: str, stdin: str | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=tree, input=stdin, capture_output=True, text=True,
                          check=False, errors="surrogateescape")


def changed_paths(diff: str) -> list[str]:
    """Every path a unified diff touches (old and new names)."""
    paths = set()
    for line in diff.splitlines():
        if line.startswith(("--- a/", "+++ b/")):
            paths.add(line[6:].split("\t")[0])
    return sorted(paths)


def package_of(tree: Path, path: str) -> str | None:
    """The Cargo package owning `path` in `tree`: the nearest Cargo.toml with a [package] under crates/."""
    parts = path.split("/")
    if parts[0] != "crates":
        return None
    for depth in range(len(parts) - 1, 1, -1):
        manifest = tree.joinpath(*parts[:depth], "Cargo.toml")
        if manifest.is_file():
            package = tomllib.loads(manifest.read_text()).get("package")
            return package.get("name") if package else None
    return None


def _lock_rows(tree: Path) -> list[str]:
    path = tree / LOCK_REL
    return [line for line in path.read_text().splitlines() if line.strip()] if path.is_file() else []


def _packages(tree: Path, diff: str) -> set[str]:
    return {name for path in changed_paths(diff) if (name := package_of(tree, path))}


def _package_exists(tree: Path, name: str) -> bool:
    for manifest in (tree / "crates").glob("**/Cargo.toml"):
        package = tomllib.loads(manifest.read_text()).get("package")
        if package and package.get("name") == name:
            return True
    return False


def _listing(tree: Path, packages: set[str] | None, lister: Lister) -> dict[str, bool]:
    if packages is None:
        return lister(tree, None)
    present = sorted(name for name in packages if _package_exists(tree, name))
    return lister(tree, present) if present else {}


def workspace_wide(diff: str) -> bool:
    """A `Cargo.toml` or a path outside `crates/` can change which tests any package compiles or runs."""
    return any(not path.startswith("crates/") or path.rsplit("/", 1)[-1] == "Cargo.toml"
               for path in changed_paths(diff))


def _measure_pair(tree: Path, diff: str, lister: Lister) -> tuple[dict, dict] | None:
    """(baseline, after): `tree` with `diff` reversed, and `tree`; None when the diff does not reverse.

    Both list the packages the diff touches in either tree (a package the fix deletes loses its
    tests, one it adds only adds tests), or the whole workspace (`workspace_wide`).
    """
    if _git(tree, "apply", "-R", "--check", "--whitespace=nowarn", "-", stdin=diff).returncode:
        return None
    touched = None if workspace_wide(diff) else _packages(tree, diff)
    after = {"tests": _listing(tree, touched, lister), "lock": _lock_rows(tree)}
    reversed_ = _git(tree, "apply", "-R", "--whitespace=nowarn", "-", stdin=diff)
    if reversed_.returncode:
        raise GuardRefused(f"reversing the fix on {tree} failed: {reversed_.stderr.strip()}")
    if touched is not None:
        touched |= _packages(tree, diff)
    baseline = {"tests": _listing(tree, touched, lister), "lock": _lock_rows(tree)}
    return baseline, dict(after, packages="workspace" if touched is None else sorted(touched))


def _lock_problems(baseline: list[str], after: list[str]) -> tuple[list[str], dict]:
    kept, before = set(after), set(baseline)
    removed = [line for line in baseline if line not in kept]
    known = {(row["mc"], row["case"]) for row in map(json.loads, baseline)}
    added = [json.loads(line) for line in after if line not in before]
    rerecorded = [f"{row['case']} v{row['version']}" for row in added if (row["mc"], row["case"]) in known]
    problems = []
    if removed:
        problems.append(f"removes {len(removed)} recorded-corpus lock line(s) ({LOCK_REL})")
    if rerecorded:
        problems.append(f"changes the replayed version of recorded case(s) {rerecorded}: a re-record is its own "
                        "commit with `re-record --reason`, never part of a fix")
    return problems, {"removed": removed, "rerecorded": rerecorded,
                      "added_cases": sorted({row["case"] for row in added} - {case for _, case in known})}


def promotion_guard(tree: Path, fix_diff: str, lister: Lister, *, patch: bool) -> dict:
    """Compare `tree` (the promoted tree) with itself minus the whole fix; refuse lost tests or corpus edits.

    `tree` is left with the fix reversed: the caller restores it.
    """
    pair = _measure_pair(tree, fix_diff, lister)
    if pair is None:
        raise GuardRefused(
            "the fix patch does not reverse as a whole on the candidate: every hunk, test hunks included, must be "
            "what that tree holds" if patch else "fix no longer reverses on the candidate; use patch: mode")
    baseline, after = pair
    lost = sorted(name for name, ignored in baseline["tests"].items() if not ignored and name not in after["tests"])
    ignored = sorted(name for name, was in baseline["tests"].items()
                     if not was and after["tests"].get(name) is True)
    problems = []
    if lost:
        problems.append(f"drops {len(lost)} test(s) the tree before it runs (test identity changed: "
                        + ", ".join(f"{name} -> missing" for name in lost[:10])
                        + "; land the move as its own commit)")
    if ignored:
        problems.append(f"ignores {len(ignored)} test(s) the tree before it runs: {ignored[:10]}")
    lock_problems, lock = _lock_problems(baseline["lock"], after["lock"])
    problems += lock_problems
    record = {"packages": after["packages"],
              "tests": {"baseline": len(baseline["tests"]), "after": len(after["tests"]), "lost": lost,
                        "newly_ignored": ignored,
                        "added": sorted(set(after["tests"]) - set(baseline["tests"]))},
              "lock": lock, "verdict": "refused" if problems else "pass", "problems": problems}
    if problems:
        raise GuardRefused("the fix " + "; ".join(problems) + ": a fix may not pass by deleting, disabling or "
                           "ignoring tests or by changing the recorded corpus; test removals belong in a separate, "
                           "listed test-prune commit")
    return record
