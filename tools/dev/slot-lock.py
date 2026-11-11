#!/usr/bin/env python3
"""Run a command holding one of N shared host slots (flock), whichever frees first.

The one slot mechanism on the owner host: scene-slot.sh (six graphical-gate slots)
and mem-run.sh (three build tokens) both run through it.

    slot-lock.py --dir DIR --name FMT --slots a,b,c --env VAR [--fd N]
                 [--wait-dir DIR] [--wait-message TEXT] -- COMMAND [ARGS...]

- Slot <s> is an exclusive flock on DIR/(FMT % s). A free slot is taken at once, in
  --slots order. Otherwise the waiter blocks on every slot at once (one thread per
  slot) and takes whichever frees first, so one long holder never starves the queue.
  No polling: a waiter sleeps in flock and uses no CPU.
- The won lock is inherited by COMMAND (exec keeps the pid), so the slot is held as
  long as that process, and whatever it passes the fd to, lives: any exit, kill or
  OOM returns it. It sits at fd N (--fd; mem-run.sh uses 9) or at the lowest free
  fd >= 10 (mem-run.sh reopens and closes fds 3-9, which would release a lower one).
- COMMAND sees VAR=<slot> and VAR_PID=<its own pid>.
- The holder writes one JSON line into its lock file (pid, since, cwd, argv); readers
  such as fleet-status trust it only while the lock is held.
- A waiter prints TEXT to stderr, records itself as --wait-dir/<pid>.json until it
  wins (stale records of dead waiters are dropped by the next waiter), and, when the
  environment names a pipe fd in VAR_READY_FD (tests), writes b"w" to it once it is
  queued; a caller that won at once gets the fd closed instead.
"""

from __future__ import annotations

import fcntl
import json
import os
import signal
import sys
import threading
import time
from pathlib import Path


def usage(message: str) -> None:
    sys.exit(f"slot-lock.py: {message}\nusage: slot-lock.py --dir DIR --name FMT --slots a,b,c --env VAR "
             "[--fd N] [--wait-dir DIR] [--wait-message TEXT] -- COMMAND [ARGS...]")


def parse(argv: list[str]) -> tuple[dict[str, str], list[str]]:
    if "--" not in argv:
        usage("missing -- before the command")
    split = argv.index("--")
    options, command = argv[:split], argv[split + 1:]
    known = {"--dir", "--name", "--slots", "--env", "--fd", "--wait-dir", "--wait-message"}
    opts: dict[str, str] = {}
    for i in range(0, len(options), 2):
        if options[i] not in known or i + 1 >= len(options):
            usage(f"bad option {options[i]!r}")
        opts[options[i]] = options[i + 1]
    missing = [name for name in ("--dir", "--name", "--slots", "--env") if name not in opts]
    if missing or not command:
        usage(f"missing {' '.join(missing) or 'command'}")
    return opts, command


def pid_alive(pid: int) -> bool:
    return Path(f"/proc/{pid}").exists()


def register_wait(wait_dir: Path, record: bytes) -> Path:
    wait_dir.mkdir(parents=True, exist_ok=True)
    for old in wait_dir.glob("*.json"):
        if old.stem.isdigit() and not pid_alive(int(old.stem)):
            old.unlink(missing_ok=True)
    path = wait_dir / f"{os.getpid()}.json"
    tmp = wait_dir / f".{os.getpid()}.tmp"
    tmp.write_bytes(record)
    tmp.replace(path)
    return path


def main() -> None:
    opts, command = parse(sys.argv[1:])
    lock_dir = Path(opts["--dir"])
    lock_dir.mkdir(parents=True, exist_ok=True)
    slots = [slot for slot in opts["--slots"].split(",") if slot]
    var = opts["--env"]
    fixed_fd = int(opts["--fd"]) if "--fd" in opts else None
    ready = os.environ.pop(f"{var}_READY_FD", None)
    if fixed_fd is not None:
        if ready and int(ready) == fixed_fd:
            ready = str(os.dup(fixed_fd))
            os.close(fixed_fd)
        # Reserve the target number first, so no slot file is opened on it.
        placeholder = os.open("/dev/null", os.O_RDONLY)
        os.dup2(placeholder, fixed_fd, inheritable=False)
        os.close(placeholder)
    try:
        cwd = os.getcwd()
    except OSError:
        cwd = os.environ.get("PWD", "?")
    record = (json.dumps({"pid": os.getpid(), "since": int(time.time()), "cwd": cwd, "argv": command}) + "\n").encode()

    def open_slot(slot: str) -> int:
        # Non-inheritable (PEP 446): a losing slot is released at exec at the latest.
        return os.open(lock_dir / (opts["--name"] % slot), os.O_WRONLY | os.O_CREAT, 0o666)

    chosen: list[tuple[int, str]] = []
    for slot in slots:  # every slot file exists, so readers can count free slots
        fd = open_slot(slot)
        if not chosen:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                chosen.append((fd, slot))
                continue
            except BlockingIOError:
                pass
        os.close(fd)

    wait_file = None
    if chosen:
        if ready:
            os.close(int(ready))
    else:
        if "--wait-message" in opts:
            print(opts["--wait-message"], file=sys.stderr, flush=True)
        if "--wait-dir" in opts:
            wait_file = register_wait(Path(opts["--wait-dir"]), record)

        def give_up(signum, _frame):
            if wait_file:
                wait_file.unlink(missing_ok=True)
            os._exit(128 + signum)

        for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            signal.signal(signum, give_up)
        guard, won = threading.Lock(), threading.Event()

        def take(slot: str) -> None:
            fd = open_slot(slot)
            fcntl.flock(fd, fcntl.LOCK_EX)
            with guard:
                if chosen:
                    os.close(fd)
                    return
                chosen.append((fd, slot))
            won.set()

        for slot in slots:
            threading.Thread(target=take, args=(slot,), daemon=True).start()
        if ready:
            os.write(int(ready), b"w")
            os.close(int(ready))
        won.wait()
    fd, slot = chosen[0]
    os.ftruncate(fd, 0)
    os.pwrite(fd, record, 0)
    if wait_file:
        wait_file.unlink(missing_ok=True)
    if fixed_fd is not None:
        os.dup2(fd, fixed_fd, inheritable=True)
    else:
        fcntl.fcntl(fd, fcntl.F_DUPFD, 10)  # inheritable copy; the original closes at exec
    os.environ[var] = slot
    os.environ[f"{var}_PID"] = str(os.getpid())
    try:
        os.execvp(command[0], command)
    except OSError as err:
        sys.exit(f"slot-lock.py: cannot run {command[0]}: {err}")


if __name__ == "__main__":
    main()
