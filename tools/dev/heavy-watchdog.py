#!/usr/bin/env python3
"""Memory watchdog for mem-run heavy commands (owner 2026-10-28: no queue, preempt).

Every mem-run command starts at once in its own scope under solaris-heavy.slice and
registers `<dir>/run/<pid>` = "<cgroup>\t<expected_peak_kib>\t<preemptible 0|1>".
This service owns freezing (cgroup v2 freezer: paused, not killed). Only commands
mem-run registered as preemptible are ever frozen: plain `cargo build/check/clippy/
doc` and `cargo test --no-run`, decided once from argv. Tests, harness runs,
servers and clients are never frozen, so their timeouts stay honest.
- Only real memory shortage freezes (no prediction at start): a PSI trigger on the
  slice (its tasks stalling on reclaim) or the slice hitting its MemoryMax freezes the youngest
  preemptible command, one per event, but only while the room left (below) is under the
  4G reserve; the oldest running command is never frozen, so something always finishes.
  Builds fill the slice with clean page cache that the kernel drops at the cap, so
  hitting MemoryMax or a short reclaim stall alone is not a shortage.
- When a command exits it thaws frozen commands oldest first while their expected
  remaining growth (measured peaks) fits the room: MemAvailable minus a 4G reserve,
  and the slice cap minus its unreclaimable memory (memory.current without clean page
  cache; tmpfs, dirty and writeback pages count). With nothing running it thaws the
  oldest frozen one regardless.
- While `<dir>/exclusive` names a live mem-run -x, it freezes every preemptible command,
  thaws nothing and writes `<dir>/quiet` once every registered command is frozen.

Event driven, no polling: wakeups come from the FIFO `<dir>/events` (mem-run writes a
line on start/exit/exclusive), pidfds of registered commands, the slice's PSI trigger
and its memory.events.
"""
from __future__ import annotations

import os
import select
import stat
import sys
from pathlib import Path

DIR = Path(sys.argv[1])
SLICE = Path(sys.argv[2])  # /sys/fs/cgroup/<slice cgroup>
RESERVE_KIB = 4 * 1024 * 1024
# 300 ms of stalled memory time within 2 s. Unprivileged PSI triggers need a window
# that is a multiple of 2 s.
PSI_TRIGGER = b"some 300000 2000000"
# Shared jobserver tokens: one per logical CPU.
JOBS = os.cpu_count() or 4
JOBSERVER = -1


def log(message: str) -> None:
    print(message, flush=True)


def read(path: Path) -> str | None:
    try:
        return path.read_text()
    except OSError:
        return None


def avail_kib() -> int:
    for line in (read(Path("/proc/meminfo")) or "").splitlines():
        if line.startswith("MemAvailable:"):
            return int(line.split()[1])
    return 0


def alive(pid: str) -> bool:
    """Running, not a zombie: an exited but unreaped wrapper already fired its pidfd."""
    stat = read(Path(f"/proc/{pid}/stat")) if pid.isdigit() else None
    return bool(stat) and stat[stat.rindex(")") + 2] != "Z"


POLLER = select.poll()
WATCHED: dict[int, int] = {}  # pidfd -> pid of a registered command or -x window
EVENT_FDS: dict[int, Path] = {}  # cgroup.events fd -> scope awaiting freeze confirmation


def watch(pid: str) -> None:
    """Wake on this process's exit even if it dies without writing an exit event."""
    if int(pid) in WATCHED.values():
        return
    try:
        fd = os.pidfd_open(int(pid))
    except OSError:
        return
    WATCHED[fd] = int(pid)
    POLLER.register(fd, select.POLLIN)


def commands() -> list[dict]:
    """Live registered commands, oldest first; stale registrations are removed."""
    rows = []
    for entry in (DIR / "run").iterdir():
        # The wrapper removes its registration on exit and writes it non-atomically.
        try:
            started = entry.stat().st_mtime_ns
        except FileNotFoundError:
            continue
        parts = (read(entry) or "").rstrip("\n").split("\t")
        if not alive(entry.name):
            # A killed wrapper can leave its build processes in a frozen scope that
            # nobody would thaw again.
            if parts[0] and (read(Path("/sys/fs/cgroup" + parts[0]) / "cgroup.freeze") or "").strip() == "1":
                try:
                    (Path("/sys/fs/cgroup" + parts[0]) / "cgroup.freeze").write_text("0")
                    log(f"thaw orphaned scope of dead pid {entry.name}")
                except OSError:
                    pass
            entry.unlink(missing_ok=True)
            continue
        watch(entry.name)
        if len(parts) < 2 or not parts[1].isdigit():
            continue
        cg = Path("/sys/fs/cgroup" + parts[0])
        current = read(cg / "memory.current")
        if current is None:
            continue
        rows.append({
            "pid": entry.name, "cg": cg, "est": int(parts[1]),
            # Registrations of the previous runner version have no flag: never touch them.
            "preemptible": len(parts) > 2 and parts[2] == "1",
            "target": parts[3] if len(parts) > 3 else "",
            "cur": int(current) // 1024,
            "frozen": (read(cg / "cgroup.freeze") or "0").strip() == "1",
            "started": started,
        })
    return sorted(rows, key=lambda row: row["started"])


def growth(row: dict) -> int:
    return max(0, row["est"] - row["cur"])


def set_frozen(row: dict, frozen: bool, why: str) -> None:
    try:
        (row["cg"] / "cgroup.freeze").write_text("1" if frozen else "0")
        row["frozen"] = frozen
        log(f"{'freeze' if frozen else 'thaw'} pid={row['pid']} expect={row['est'] // 1024}M "
            f"now={row['cur'] // 1024}M: {why}")
    except OSError as error:
        log(f"cannot {'freeze' if frozen else 'thaw'} pid={row['pid']}: {error}")


def exclusive_window() -> bool:
    pid = (read(DIR / "exclusive") or "").strip()
    if pid and alive(pid):
        watch(pid)
        return True
    if pid:
        (DIR / "exclusive").unlink(missing_ok=True)
        log(f"exclusive window of dead pid {pid} cleared")
    return False


def unreclaimable_kib(cg: Path) -> int:
    """memory.current minus the clean page cache the kernel drops at the cap."""
    fields = dict(line.split() for line in (read(cg / "memory.stat") or "").splitlines())
    cache = (int(fields.get("file", 0)) - int(fields.get("shmem", 0))
             - int(fields.get("file_dirty", 0)) - int(fields.get("file_writeback", 0)))
    return (int(read(cg / "memory.current") or 0) - max(0, cache)) // 1024


def headroom_kib() -> int:
    slice_max = (read(SLICE / "memory.max") or "max").strip()
    room = avail_kib() - RESERVE_KIB
    if slice_max != "max":
        room = min(room, int(slice_max) // 1024 - unreclaimable_kib(SLICE))
    return room


def reset_tokens() -> None:
    """Refill the shared jobserver to exactly JOBS tokens (only when no command runs,
    so tokens a killed cargo never returned do not leak forever)."""
    while True:
        try:
            if not os.read(JOBSERVER, 4096):
                break
        except BlockingIOError:
            break
    os.write(JOBSERVER, b"+" * JOBS)


def balance(pressure: bool) -> None:
    rows = commands()
    if not rows:
        reset_tokens()
    if exclusive_window():
        for row in rows:
            if not row["frozen"] and row["preemptible"]:
                set_frozen(row, True, "exclusive -x window")
        # cgroup.freeze is only the request; -x starts once cgroup.events confirms it.
        # Watch first, then check, so a confirmation in between still wakes us.
        for row in rows:
            if row["frozen"]:
                watch_events(row["cg"])
        if all(row["frozen"] and frozen_confirmed(row) for row in rows):
            try:
                quiet = os.open(DIR / "quiet", os.O_WRONLY | os.O_NONBLOCK)
                os.write(quiet, b"quiet\n")
                os.close(quiet)
            except OSError:
                pass
        return
    if not pressure or headroom_kib() >= RESERVE_KIB:
        thaw(rows)
        return
    running = [row for row in rows if not row["frozen"]]
    # Youngest first; the oldest running command always keeps running. A command whose
    # target dir another running command uses is skipped: freezing the holder of
    # cargo's build lock would stall the other one too.
    for row in reversed(running[1:]):
        shared = row["target"] and any(o is not row and o["target"] == row["target"] for o in running)
        if row["preemptible"] and not shared:
            set_frozen(row, True, f"memory pressure, {avail_kib() // 1024}M available")
            return


def thaw(rows: list[dict]) -> None:
    room = headroom_kib() - sum(growth(row) for row in rows if not row["frozen"])
    for row in rows:
        if not row["frozen"]:
            continue
        if growth(row) <= room or not any(not other["frozen"] for other in rows):
            set_frozen(row, False, "memory available" if growth(row) <= room else "nothing else running")
            room -= growth(row)


def frozen_confirmed(row: dict) -> bool:
    return "frozen 1" in (read(row["cg"] / "cgroup.events") or "")


def watch_events(cg: Path) -> None:
    """Wake when this scope's cgroup.events changes (freeze completes)."""
    if cg in EVENT_FDS.values():
        return
    try:
        fd = os.open(cg / "cgroup.events", os.O_RDONLY | os.O_NONBLOCK)
        os.read(fd, 4096)
    except OSError:
        return
    EVENT_FDS[fd] = cg
    POLLER.register(fd, select.POLLPRI)


def main() -> None:
    (DIR / "run").mkdir(parents=True, exist_ok=True)
    fifo = DIR / "events"
    if not fifo.exists():
        os.mkfifo(fifo)
    # Read-write: the FIFO never reports EOF between writers.
    events = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
    # One make-style jobserver for every cargo under mem-run (CARGO_MAKEFLAGS): all
    # builds together run about JOBS compilers instead of JOBS per build.
    global JOBSERVER
    jobserver = DIR / "jobserver"
    if jobserver.exists() and not stat.S_ISFIFO(jobserver.stat().st_mode):
        jobserver.unlink()
    if not jobserver.exists():
        os.mkfifo(jobserver)
    JOBSERVER = os.open(jobserver, os.O_RDWR | os.O_NONBLOCK)
    reset_tokens()
    # Slice PSI also counts stalls its tasks suffer from host-wide shortage (the host
    # pressure file refuses unprivileged triggers).
    psi = os.open(SLICE / "memory.pressure", os.O_RDWR | os.O_NONBLOCK)
    os.write(psi, PSI_TRIGGER)
    # Changes when the slice hits MemoryMax (max/oom counters).
    slice_events = os.open(SLICE / "memory.events", os.O_RDONLY | os.O_NONBLOCK)
    os.read(slice_events, 4096)
    POLLER.register(events, select.POLLIN)
    POLLER.register(psi, select.POLLPRI)
    POLLER.register(slice_events, select.POLLPRI)
    log(f"watching {DIR} slice={SLICE}")
    balance(False)
    while True:
        for fd, _ in POLLER.poll():
            if fd == events:
                try:
                    os.read(events, 65536)
                except BlockingIOError:
                    pass
                balance(False)
            elif fd in WATCHED or fd in EVENT_FDS:
                POLLER.unregister(fd)
                os.close(fd)
                WATCHED.pop(fd, None)
                EVENT_FDS.pop(fd, None)
                balance(False)
            else:
                if fd == slice_events:
                    # kernfs keeps reporting the change until the file is read again.
                    os.lseek(slice_events, 0, os.SEEK_SET)
                    os.read(slice_events, 4096)
                balance(True)


if __name__ == "__main__":
    main()
