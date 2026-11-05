#!/usr/bin/env python3
"""Run trusted instrumentation in a private tmpfs under mem-run and Landlock.

Usage: python3 instrumentation-guard.py -- COMMAND [ARG ...]
Run from the desired workspace. The printed private /dev/shm directory contains
run.log, target, tmp, profiles and a Cargo home. Registry/git caches are symlinked
read-only through Landlock. Outputs are deliberately not copied to games disk.
This confines this command's filesystem writes, not other disk users or external
services contacted by the command. Keep the run directory to inspect failures;
remove only your own directory once its artifacts are no longer needed.
"""
import ctypes
import os
from pathlib import Path
import subprocess
import sys
import tempfile

MEM_RUN = str(Path(__file__).resolve().with_name("mem-run.sh"))
MAX_MEMORY = 6 * 1024**3
WRITE_RIGHTS = (1 << 1) | sum(1 << bit for bit in range(4, 15))
libc = ctypes.CDLL(None, use_errno=True)


def checked(result, operation):
    if result < 0:
        raise OSError(ctypes.get_errno(), operation)
    return result


def confine(directory):
    # ABI 3 adds TRUNCATE; deny creation/removal/reparenting and writes everywhere
    # except this directory. Read/execute remain unrestricted.
    if checked(libc.syscall(444, 0, 0, 1), "Landlock ABI") < 3:
        raise RuntimeError("Landlock ABI 3 or newer required")
    rights = ctypes.c_uint64(WRITE_RIGHTS)
    ruleset = checked(libc.syscall(444, ctypes.byref(rights), ctypes.sizeof(rights), 0), "create ruleset")
    class PathRule(ctypes.Structure):
        _layout_ = "ms"
        _pack_ = 1
        _fields_ = [("allowed", ctypes.c_uint64), ("parent_fd", ctypes.c_int32)]
    path_fd = os.open(directory, os.O_PATH | os.O_CLOEXEC)
    try:
        rule = PathRule(WRITE_RIGHTS, path_fd)
        null_fd = os.open("/dev/null", os.O_PATH | os.O_CLOEXEC)
        try:
            null_rule = PathRule(1 << 1, null_fd)
            checked(libc.syscall(445, ruleset, 1, ctypes.byref(null_rule), 0), "allow /dev/null")
        finally:
            os.close(null_fd)
        checked(libc.syscall(445, ruleset, 1, ctypes.byref(rule), 0), "add path rule")
        checked(libc.prctl(38, 1, 0, 0, 0), "set no_new_privs")
        checked(libc.syscall(446, ruleset, 0), "restrict self")
    finally:
        os.close(path_fd)
        os.close(ruleset)


def child(directory, command):
    # The outer mem-run scope must exist and have a real hard memory limit.
    cgroup = next(line.split("::", 1)[1].strip() for line in Path("/proc/self/cgroup").read_text().splitlines() if line.startswith("0::"))
    limit = Path("/sys/fs/cgroup", cgroup.lstrip("/"), "memory.max").read_text().strip()
    if limit == "max" or int(limit) > MAX_MEMORY:
        raise RuntimeError(f"missing <=6 GiB cgroup memory limit: {limit}")
    root = Path(directory).resolve(strict=True)
    if root.parent != Path("/dev/shm").resolve() or not root.name.startswith("solaris-instrument-"):
        raise RuntimeError("invalid private tmpfs directory")
    env = os.environ.copy()
    env.update(CARGO_HOME=str(root / "cargo"), CARGO_TARGET_DIR=str(root / "target"),
               TMPDIR=str(root / "tmp"), LLVM_PROFILE_FILE=str(root / "profiles" / "%p-%m.profraw"),
               RUSTC_WRAPPER="", CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2")
    env.pop("RUSTC_WORKSPACE_WRAPPER", None)
    # The host jobserver FIFO is outside the allowed tree.
    for name in ("CARGO_MAKEFLAGS", "MAKEFLAGS", "MFLAGS"):
        env.pop(name, None)
    # Prevent inherited descriptors from bypassing Landlock. stdout/stderr are
    # opened by the parent on tmpfs; stdin is /dev/null.
    os.closerange(3, os.sysconf("SC_OPEN_MAX"))
    confine(root)
    os.execvpe(command[0], command, env)


def main():
    if len(sys.argv) >= 4 and sys.argv[1] == "--child":
        child(sys.argv[2], sys.argv[3:])
    if len(sys.argv) < 3 or sys.argv[1] != "--":
        raise SystemExit(__doc__)
    root = Path(tempfile.mkdtemp(prefix="solaris-instrument-", dir="/dev/shm"))
    for part in ("cargo", "target", "tmp", "profiles"):
        (root / part).mkdir()
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
    for part in ("registry", "git"):
        source = cargo_home / part
        if source.exists():
            (root / "cargo" / part).symlink_to(source, target_is_directory=True)
    print(f"instrumentation-guard: output={root}", flush=True)
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(root / "target")
    with open(root / "run.log", "xb", buffering=0) as log:
        result = subprocess.run([MEM_RUN, "-m", "6G", sys.executable, str(Path(__file__).resolve()),
                                 "--child", str(root), *sys.argv[2:]],
                                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                                close_fds=True, env=env)
    print(f"instrumentation-guard: exit={result.returncode} log={root / 'run.log'}", flush=True)
    return result.returncode if result.returncode >= 0 else 128 - result.returncode


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, StopIteration, ValueError) as exc:
        print(f"instrumentation-guard: {exc}", file=sys.stderr)
        sys.exit(1)
