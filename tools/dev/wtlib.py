"""Shared helpers for the owner-host worktree and release-pin tools.

Used by wt-new, wt-done, wt-gc and pin-release; stdlib only. Everything is
anchored at the owner host's work root, so these tools only make sense there.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

WORK = Path("/run/media/kaiserroman/games/solaris-work")
REPOS = {
    "core": WORK / "solaris",
    "client": WORK / "solaris-client",
    "loader": WORK / "solaris-loader",
    "plugins": WORK / "solaris-default-plugins",
}
WT = WORK / "wt"
TARGETS = WORK / "targets"
HOMES = WORK / "wt-home"
RECEIPTS = WORK / "receipts"
PINS = WORK / "pins"
TMP = WORK / "tmp"
MEM_RUN = REPOS["core"] / "tools/dev/mem-run.sh"
TOOLS = Path(__file__).resolve().parent


class ToolError(Exception):
    """A refusal or failure with a message meant for the user."""


def run_tool(main) -> None:
    """Run `main()`; print a ToolError as `<tool>: <message>` and exit 1."""
    try:
        main()
    except ToolError as err:
        sys.exit(f"{Path(sys.argv[0]).name}: {err}")
    except KeyboardInterrupt:
        sys.exit(130)


def check_host() -> None:
    if not (REPOS["core"] / ".git").exists():
        raise ToolError(f"not the owner host: {REPOS['core']} is not a checkout")


def git(cwd, *args, check=True, env=None, binary=False) -> str | bytes:
    """Run git in `cwd`; return stdout (str without the trailing newline, or bytes)."""
    proc = subprocess.run(
        ["git", "-C", str(cwd), *args],
        capture_output=True,
        env=None if env is None else {**os.environ, **env},
    )
    if check and proc.returncode != 0:
        err = proc.stderr.decode(errors="replace").strip()
        raise ToolError(f"git {' '.join(args[:3])} failed in {cwd}: {err}")
    if binary:
        return proc.stdout
    return proc.stdout.decode(errors="replace").rstrip("\n")


def git_ok(cwd, *args) -> bool:
    return subprocess.run(["git", "-C", str(cwd), *args], capture_output=True).returncode == 0


def repo_of_worktree(path: Path) -> str:
    """Key in REPOS of the repository a worktree belongs to."""
    common = Path(git(path, "rev-parse", "--path-format=absolute", "--git-common-dir")).resolve()
    for key, repo in REPOS.items():
        if (repo / ".git").resolve() == common:
            return key
    raise ToolError(f"{path} belongs to {common}, which is none of the four Solaris repositories")


def worktree_entries(key: str) -> list[dict]:
    """Registered linked worktrees of a repo (the main checkout is skipped):
    dicts with path, head, branch (None when detached), locked, prunable."""
    entries, current = [], None
    for line in str(git(REPOS[key], "worktree", "list", "--porcelain")).splitlines() + [""]:
        if line.startswith("worktree "):
            current = {"path": Path(line[9:]), "head": "", "branch": None, "locked": False, "prunable": False}
        elif current is None:
            continue
        elif line.startswith("HEAD "):
            current["head"] = line[5:]
        elif line.startswith("branch "):
            current["branch"] = line[7:].removeprefix("refs/heads/")
        elif line.startswith("locked"):
            current["locked"] = True
        elif line.startswith("prunable"):
            current["prunable"] = True
        elif line == "":
            entries.append(current)
            current = None
    return entries[1:]


def default_branch_ref(key: str) -> str:
    """The ref `commits not in main` is measured against: refs/heads/main, else the checkout's HEAD."""
    repo = REPOS[key]
    if git_ok(repo, "rev-parse", "--verify", "--quiet", "refs/heads/main"):
        return "refs/heads/main"
    return str(git(repo, "rev-parse", "HEAD"))


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def working_state(tree_dir, include_untracked: bool = True) -> tuple[str, list[str]]:
    """Tree object of the working tree's current content, and the untracked files.

    The worktree's own index is never touched: a copy of it is synchronised with
    the working tree (`git add -A`, or `-u` without untracked files) and written
    out as a tree. Ignored files are not part of it. The untracked list holds the
    non-ignored files outside the index, whether or not they are included.
    """
    top = Path(str(git(tree_dir, "rev-parse", "--show-toplevel")))
    real_index = Path(str(git(top, "rev-parse", "--path-format=absolute", "--git-path", "index")))
    with tempfile.TemporaryDirectory(prefix="wt-index-") as scratch:
        index = Path(scratch) / "index"
        env = {"GIT_INDEX_FILE": str(index)}
        if real_index.exists():
            shutil.copyfile(real_index, index)
        else:
            git(top, "read-tree", "HEAD", env=env)
        git(top, "add", "-A" if include_untracked else "-u", env=env)
        tree = str(git(top, "write-tree", env=env))
    raw = git(top, "ls-files", "--others", "--exclude-standard", "-z", binary=True)
    untracked = [item.decode(errors="surrogateescape") for item in raw.split(b"\0") if item]
    return tree, untracked


def diff_between(cwd, old: str, new: str) -> bytes:
    """Binary-safe, rename-free unified diff old..new with a/ b/ prefixes (what `git apply` expects)."""
    return git(
        cwd, "diff", "--binary", "--full-index", "--no-ext-diff", "--no-textconv", "--no-color",
        "--no-renames", "--src-prefix=a/", "--dst-prefix=b/", old, new, binary=True,
    )


def dir_size_bytes(path: Path) -> int:
    if not path.exists():
        return 0
    out = subprocess.run(["du", "-sk", "--", str(path)], capture_output=True, text=True)
    try:
        return int(out.stdout.split()[0]) * 1024
    except (IndexError, ValueError):
        return 0


def human_bytes(size: int) -> str:
    value = float(size)
    for unit in ("B", "K", "M", "G"):
        if value < 1024 or unit == "G":
            return f"{value:.0f}{unit}" if unit == "B" else f"{value:.1f}{unit}"
        value /= 1024
    return f"{size}B"


def is_under(path: Path, parent: Path) -> bool:
    try:
        path.resolve().relative_to(parent.resolve())
        return path.resolve() != parent.resolve()
    except ValueError:
        return False
