"""Promotion guard of the vanilla differential oracle (owner D128).

A fix must not make itself pass by deleting or ignoring tests, and must not edit the
recorded corpus. `ledger promote` calls `promotion_guard` for every core fix (a commit or
a `patch:` fix) before it runs any control:

- `#[test]`-family attributes, `assert!`-family macros and `#[ignore]` attributes are
  counted in the fix's own diff of Rust sources under `crates/` (comments excluded); fewer
  tests or assertions, or more `#[ignore]`s, than before the fix refuses the promotion.
  Counts span every crate, so tests a crate split moves between crates count once.
- a line removed from `corpus-lock.jsonl` refuses it: recorded cases are append-only and
  change only through a versioned re-record, never inside a fix.

The baseline is the tree before the fix (its parent commit, or the candidate with the
patch reversed: the tree whose control goes red). Its census, the fixed tree's census and
the fix's delta are returned for promote.json, so no Git-tracked count needs editing by
every commit. Test-prune commits are separate reviewed commits, never a row's fix, so they
are not checked here.
"""
from __future__ import annotations

import re
import subprocess
from pathlib import Path

TEST_ATTR = re.compile(r"#\[\s*(?:tokio::)?test\b|#\[\s*rstest\b|#\[\s*test_case\b")
IGNORE_ATTR = re.compile(r"#\[\s*ignore\b")
ASSERT_MACRO = re.compile(r"\b(?:debug_)?(?:prop_)?assert(?:_eq|_ne|_matches)?!")
GREP = r"#\[[[:space:]]*(tokio::)?test|#\[[[:space:]]*rstest|#\[[[:space:]]*test_case|#\[[[:space:]]*ignore|assert(_eq|_ne|_matches)?!"
LOCK_REL = "tools/harness/quirk_parity/corpus-lock.jsonl"
EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
METRICS = ("tests", "asserts", "ignores")


class GuardRefused(RuntimeError):
    pass


def _code(line: str) -> str:
    """The line without its `//` comment (string literals kept intact)."""
    quoted = escaped = False
    for index, char in enumerate(line):
        if escaped:
            escaped = False
        elif char == "\\" and quoted:
            escaped = True
        elif char == '"':
            quoted = not quoted
        elif char == "/" and not quoted and line.startswith("//", index):
            return line[:index]
    return line


def count(line: str) -> dict[str, int]:
    code = _code(line)
    return {"tests": len(TEST_ATTR.findall(code)), "asserts": len(ASSERT_MACRO.findall(code)),
            "ignores": len(IGNORE_ATTR.findall(code))}


def _add(total: dict[str, int], part: dict[str, int], sign: int = 1) -> None:
    for key in METRICS:
        total[key] = total.get(key, 0) + sign * part[key]


def diff_delta(diff: str) -> dict:
    """Counts added minus counts removed per crate in a unified diff, plus removed lock lines."""
    crates: dict[str, dict[str, int]] = {}
    lock_removed = 0
    path = None
    for line in diff.splitlines():
        if line.startswith("--- "):
            old = line[4:].split("\t")[0]
            path = old[2:] if old.startswith("a/") else None
            continue
        if line.startswith("+++ "):
            new = line[4:].split("\t")[0]
            if new.startswith("b/"):
                path = new[2:]
            continue
        if line.startswith(("diff ", "index ", "@@", "new file", "deleted file", "similarity", "rename ")):
            if line.startswith("diff "):
                path = None
            continue
        if path is None or line[:1] not in ("+", "-"):
            continue
        if path == LOCK_REL and line.startswith("-"):
            lock_removed += 1
        parts = path.split("/")
        if len(parts) < 3 or parts[0] != "crates" or not path.endswith(".rs"):
            continue
        _add(crates.setdefault(parts[1], dict.fromkeys(METRICS, 0)), count(line[1:]),
             1 if line.startswith("+") else -1)
    total = dict.fromkeys(METRICS, 0)
    for part in crates.values():
        _add(total, part)
    return {"total": total, "crates": {name: part for name, part in sorted(crates.items()) if any(part.values())},
            "lock_lines_removed": lock_removed}


def _git(root: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=root, capture_output=True, text=True, check=check,
                          errors="surrogateescape")


def census(root: Path, rev: str | None = None) -> dict:
    """Absolute counts over `crates/**/*.rs` at `rev`, or in the working tree (untracked included)."""
    args = ["grep", "-h", "-E", "-e", GREP]
    args += [rev] if rev else ["--untracked"]
    args += ["--", "crates/*.rs"]
    proc = _git(root, *args, check=False)
    if proc.returncode not in (0, 1):
        raise GuardRefused(f"test census failed at {rev or 'the working tree'}: {proc.stderr.strip()}")
    total = dict.fromkeys(METRICS, 0)
    for line in proc.stdout.splitlines():
        _add(total, count(line))
    return {"rev": rev or "working-tree", **total}


def promotion_guard(root: Path, fix_full: str, fix_patch_text: str | None) -> dict:
    """Refuse a core fix that drops tests or assertions, adds `#[ignore]`, or edits the corpus lock."""
    if fix_patch_text is not None:
        delta = diff_delta(fix_patch_text)
        after = census(root)
        baseline = {"rev": "working-tree minus fix patch",
                    **{key: after[key] - delta["total"][key] for key in METRICS}}
    else:
        parent = f"{fix_full}^1"
        if _git(root, "rev-parse", "--verify", "--quiet", parent, check=False).returncode:
            parent = EMPTY_TREE
        delta = diff_delta(_git(root, "diff", "--no-renames", "--no-color", parent, fix_full,
                                "--", "crates", LOCK_REL).stdout)
        baseline, after = census(root, parent), census(root, fix_full)
    problems = []
    if delta["total"]["tests"] < 0:
        problems.append(f"removes {-delta['total']['tests']} test(s)")
    if delta["total"]["asserts"] < 0:
        problems.append(f"removes {-delta['total']['asserts']} assertion(s)")
    if delta["total"]["ignores"] > 0:
        problems.append(f"adds {delta['total']['ignores']} #[ignore]")
    if delta["lock_lines_removed"]:
        problems.append(f"removes {delta['lock_lines_removed']} recorded-corpus lock line(s) ({LOCK_REL})")
    record = {"baseline": baseline, "after": after, "delta": delta,
              "verdict": "refused" if problems else "pass", "problems": problems}
    if problems:
        raise GuardRefused("the fix " + "; ".join(problems) + f" (per crate: {delta['crates']}): a fix may not "
                           "pass by deleting or ignoring tests or by editing the recorded corpus; test removals "
                           "belong in a separate, listed test-prune commit")
    return record
