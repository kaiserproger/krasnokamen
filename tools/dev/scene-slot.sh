#!/bin/sh
# Run one graphical scene-shot/gate command in one of six shared host slots.
# A waiter blocks (flock) on all six slots at once and takes whichever frees
# first, so a long run in one slot never starves the queue. No polling; the
# lock is held until the command exits (an fd >= 10 is inherited across exec).
# Six fits the 16-thread owner host: one scene (release server + native client
# + Xvfb, or a Java oracle) measured about 1.5 cores and 3.5 GB (2026-11-11).
exec python3 -c '
import fcntl, os, sys, threading

cmd = sys.argv[1:]
if not cmd:
    sys.exit("usage: scene-slot.sh <command> [args...]")
lock_dir = os.environ.get("SOLARIS_SCENE_SLOT_DIR", "/tmp")  # tests use a private dir
guard, won, chosen = threading.Lock(), threading.Event(), []

def take(slot):
    # Non-inheritable fd (PEP 446): a losing slot is released at exec at the latest.
    fd = os.open("%s/solaris-scene-slot-%s.lock" % (lock_dir, slot), os.O_WRONLY | os.O_CREAT, 0o666)
    fcntl.flock(fd, fcntl.LOCK_EX)
    with guard:
        if chosen:
            os.close(fd)
            return
        chosen.append(fd)
    won.set()

for slot in "abcdef":
    threading.Thread(target=take, args=(slot,), daemon=True).start()
# Tests pass a pipe fd and wait on it instead of polling for the queued waiter.
ready = os.environ.pop("SOLARIS_SCENE_SLOT_READY_FD", None)
if ready:
    os.write(int(ready), b"w")
    os.close(int(ready))
won.wait()
# Inheritable copy at fd >= 10: mem-run.sh reopens and closes fds 3-8 (exec 3>, 3>&-),
# which would release a low lock fd before the gate ends; bash {fd} only allocates.
fcntl.fcntl(chosen[0], fcntl.F_DUPFD, 10)
os.execvp(cmd[0], cmd)
' "$@"
