#!/bin/sh
# Run one graphical scene-shot/gate command in one of six shared host slots,
# through slot-lock.py (the same mechanism as mem-run.sh's build tokens): a waiter
# blocks on all six slots at once and takes whichever frees first, so a long run in
# one slot never starves the queue. No polling; the lock is held until the command
# exits (an inherited fd >= 10, which mem-run.sh's own fds 3-9 never touch).
# Six fits the 16-thread owner host: one scene (release server + native client
# + Xvfb, or a Java oracle) measured about 1.5 cores and 3.5 GB (2026-11-11).
# The command sees SOLARIS_SCENE_SLOT=<slot>; mem-run.sh then gives its scope a
# higher CPU weight than tests and builds, so the gate keeps its tick rate.
# Holders and waiters: tools/dev/fleet-status.
[ $# -gt 0 ] || { echo "usage: scene-slot.sh <command> [args...]" >&2; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
dir=${SOLARIS_SCENE_SLOT_DIR:-/tmp}  # tests use a private dir
exec python3 "$here/slot-lock.py" --dir "$dir" --name 'solaris-scene-slot-%s.lock' --slots a,b,c,d,e,f \
  --env SOLARIS_SCENE_SLOT --wait-dir "$dir/solaris-scene-slot-wait" \
  --wait-message "scene-slot: all six scene slots are in use; waiting for one (holders: $here/fleet-status)" \
  -- "$@"
