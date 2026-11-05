#!/bin/sh
# Run one graphical scene-shot/gate command in one of three shared host slots.
# Blocks (flock) instead of polling; the lock is held until the command exits.
for s in a b c; do
  exec 9>"/tmp/solaris-scene-slot-$s.lock"
  if flock -n 9; then exec "$@"; fi
done
exec 9>/tmp/solaris-scene-slot-a.lock
flock 9
exec "$@"
