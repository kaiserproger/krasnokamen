#!/usr/bin/env python3
"""The `reserved-cpus` file protocol shared by mem-run.sh and reserve-cpus (#G4, CS341).

The file is two lines: the CPU list, then the owner's `pid starttime` (field 22 of
`/proc/<pid>/stat`, which tells a live owner from a reused pid). A killed reservation
leaves the file behind; clearing it must not race a reservation written meanwhile, so
every reader and writer serializes on `reserved-cpus.lock` and a removal re-reads the
file under that lock: only the exact bytes that were judged stale are unlinked.

    python3 tools/dev/reserved_cpus.py verdict <state-dir>   # "none" | "keep <cpus>" | "stale <cpus>"
    python3 tools/dev/reserved_cpus.py clear <state-dir>     # remove the corpse we judged, if unchanged

Install order on a host: `mem-run-guard.py` and this helper first, then `mem-run.sh`, then
`reserve-cpus`. A wrapper that cannot run this helper names it, honours the reservation file's
first line as a CPU list and refuses (exit 75) only when that line is not one; an older wrapper
ignores the owner line and honours any file it finds, so only the new three are race-safe together.
"""
from __future__ import annotations

import fcntl
import re
import sys
from pathlib import Path

FILE = 'reserved-cpus'
LOCK = 'reserved-cpus.lock'
CPU_LIST = re.compile(r'[0-9]+(?:-[0-9]+)?(?:,[0-9]+(?:-[0-9]+)?)*')


def parse(text: str) -> tuple[str | None, str | None]:
    """(cpus, owner) of a reservation file; cpus is None when the first line is not a CPU list."""
    lines = text.splitlines()
    if not lines or not CPU_LIST.fullmatch(lines[0].strip()):
        return None, None
    owner = lines[1].strip() if len(lines) > 1 and lines[1].strip() else None
    return lines[0].strip(), owner


def owner_alive(owner: str | None) -> bool | None:
    """True/False for a live/dead owner; None only when there is no owner line (an older writer).

    A present line that cannot be parsed claims a live owner and cannot be checked, so it is treated
    as stale: keeping it would pin the CPUs for as long as the file survives, and clear() unlinks only
    the exact bytes judged, so a live writer is never raced."""
    if owner is None:
        return None
    parts = owner.split()
    if len(parts) != 2 or not parts[0].isdigit():
        return False
    pid, start = parts
    try:
        text = Path(f'/proc/{pid}/stat').read_text()
    except OSError:
        return False
    fields = text.rsplit(')', 1)[-1].split()
    return len(fields) > 19 and fields[19] == start


def verdict(text: str) -> str:
    """What to do with a reservation file's text: `none`, `keep <cpus>` or `stale <cpus>`."""
    cpus, owner = parse(text)
    if cpus is None:
        return 'none'
    return f'keep {cpus}' if owner_alive(owner) is not False else f'stale {cpus}'


def read_verdict(state: Path) -> str:
    try:
        return verdict((state / FILE).read_text())
    except OSError:
        return 'none'


def unlink_if_unchanged(state: Path, judged: str) -> bool:
    """Remove the file only when it still holds the bytes that were judged; True when removed."""
    path = state / FILE
    try:
        if path.read_text() == judged:
            path.unlink()
            return True
    except OSError:
        pass
    return False


def clear(state: Path) -> str:
    """Judge and, if stale, remove under the shared lock. Returns what it did, for stderr."""
    state.mkdir(parents=True, exist_ok=True)
    with open(state / LOCK, 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            try:
                judged = (state / FILE).read_text()
            except OSError:
                return ''
            result = verdict(judged)
            if not result.startswith('stale '):
                return ''
            if unlink_if_unchanged(state, judged):
                return f"clearing stale reservation {result.removeprefix('stale ')}"
            return f"kept the reservation written while {result.removeprefix('stale ')} was judged"
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in ('verdict', 'clear'):
        print('usage: reserved_cpus.py verdict|clear <state-dir>', file=sys.stderr)
        return 2
    state = Path(argv[2])
    if argv[1] == 'verdict':
        print(read_verdict(state))
        return 0
    message = clear(state)
    if message:
        print(message, file=sys.stderr)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
