#!/bin/sh
# Run one graphical scene-shot/gate command in one of six shared host slots,
# through slot-lock.py (the same mechanism as mem-run.sh's build tokens): a waiter
# blocks on all six slots at once and takes whichever frees first, so a long run in
# one slot never starves the queue. No polling; the lock is held until the command
# exits (an inherited fd >= 10, which mem-run.sh's own fds 3-9 never touch).
# Six fits the 16-thread owner host: one scene (release server + native client
# + Xvfb, or a Java oracle) measured about 1.5 cores and 3.5 GB (2026-11-11).
# The command sees KRASNOKAMEN_SCENE_SLOT=<slot>; mem-run.sh then gives its scope a
# higher CPU weight than tests and builds, so the gate keeps its tick rate.
# Holders and waiters: tools/dev/fleet-status.
# A command that already holds a slot (a descendant of KRASNOKAMEN_SCENE_SLOT_PID) runs at
# once in that slot. Lock order: the slot comes before a mem-run -x window, never inside
# one: the window holds exclusive.lock while queued gates hold slots and wait for that lock,
# so waiting here deadlocks the fleet (2026-11-14). Wrap the window instead:
# scene-slot.sh mem-run.sh -x <command>.
[ $# -gt 0 ] || { echo "usage: scene-slot.sh <command> [args...]" >&2; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
dir=${KRASNOKAMEN_SCENE_SLOT_DIR:-/tmp}  # tests use a private dir
slot_ancestor() {
  want=${KRASNOKAMEN_SCENE_SLOT_PID:-}
  case $want in ''|*[!0-9]*) return 1 ;; esac
  p=$$
  while [ "${p:-0}" -gt 1 ]; do
    [ "$p" = "$want" ] && return 0
    p=$(awk '$1 == "PPid:" {print $2}' "/proc/$p/status" 2>/dev/null) || p=0
  done
  return 1
}
[ -n "${KRASNOKAMEN_SCENE_SLOT:-}" ] && slot_ancestor && exec "$@"
if [ -n "${KRASNOKAMEN_EXCLUSIVE_WINDOW:-}" ]; then
  echo "scene-slot: refused inside mem-run -x window $KRASNOKAMEN_EXCLUSIVE_WINDOW (lock order: take the slot first, scene-slot.sh mem-run.sh -x <command>)" >&2
  exit 75
fi
exec python3 "$here/slot-lock.py" --dir "$dir" --name 'solaris-scene-slot-%s.lock' --slots a,b,c,d,e,f \
  --env KRASNOKAMEN_SCENE_SLOT --wait-dir "$dir/solaris-scene-slot-wait" \
  --wait-message "scene-slot: all six scene slots are in use; waiting for one (holders: $here/fleet-status)" \
  -- "$@"
