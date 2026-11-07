# Sourced by mem-run.sh: choose and lock the command's Cargo target dir.
# Needs `dir` (the runner's lock dir). `select_slot "$@"` sets `target` (exporting
# CARGO_TARGET_DIR for a picked slot) and, for a pooled slot, keeps fd 7 (pool
# lock, when this command owns the slot) and fd 8 (use lock, shared) open for the
# command's whole run; mem-run's wrapper closes both for the child.
#
# A command that sets CARGO_TARGET_DIR (in its own `env ...` or in the caller's
# environment) keeps it. Otherwise it gets a free shared dir slot-a..slot-z,
# preferring the one its source tree (git toplevel of the cwd) used last; crates.io
# dependencies stay compiled across worktrees. Cargo trusts mtimes, so a tree whose
# files are older than another tree's build in the same slot would reuse that
# tree's binaries (a false green, seen 2026-11-04). On a tree switch every tracked
# file of the new tree is touched, so its workspace crates rebuild. The lock files
# a-e are the old slot locks, so commands of the previous runner version never
# share a dir with new ones. Existing dirs are reused before a new one is created;
# a new dir is created only if `df` (whole GiB, rounded up) shows at least
# SOLARIS_SLOT_MIN_FREE_GB (default 80, the owner's free-space floor) free at
# selection time (after mem-run's disk-floor slot-gc, if it ran). This only stops
# slot proliferation (12 parallel slots filled the disk on 2026-11-07); it reserves
# nothing, and existing dirs keep growing unchecked, so it is no protection against
# ENOSPC. Never waits for a free slot: with no usable dir the command is refused.
#
# Ownership of a pooled slot (<slots>/slot-<l>; its tree is <slot>/.solaris-tree):
# - Pool lock <dir>/<l>, exclusive: the command owns the slot. A picked slot is
#   always owned; an explicit CARGO_TARGET_DIR on a pooled slot takes it when free.
#   A held slot is shared only by a command of the same tree (pin-release's inner
#   build under pin-release's own lock); another tree is refused (exit 75), since
#   two trees in one slot are exactly the false green above.
# - Use lock <dir>/<l>.use: every command on the slot holds it shared for its whole
#   run. An owner holds it exclusively while it re-marks the slot, so a sharer reads
#   the tree only after the switch. A slot is picked, or taken over by another tree,
#   only when nobody holds it, so a same-tree sharer still running after its owner
#   exited keeps the slot. slot-gc deletes only while holding pool and use locks.

slot_tree() { cat "$1/.solaris-tree" 2>/dev/null; }

slot_switch_tree() {  # $1 = slot dir: re-mark it for this tree (caller holds pool + use exclusively)
  if [ "$(slot_tree "$1")" != "$tree" ]; then
    if git -C "$tree" rev-parse --git-dir >/dev/null 2>&1; then
      (cd "$tree" && git ls-files -z | xargs -0 -r touch -c --)
    fi
    mkdir -p "$1"
    printf '%s\n' "$tree" >"$1/.solaris-tree"
  fi
}

slot_refuse() {  # $1 = letter, $2 = slot dir
  echo "mem-run: slot-$1 is in use by another tree ($(slot_tree "$2")); drop CARGO_TARGET_DIR to get a free slot (exit 75)" >&2
  exit 75
}

select_slot() {
  local w l pass free_gb min_free picked= attempt real slots_real
  target=${CARGO_TARGET_DIR:-}
  for w in "$@"; do [[ $w == CARGO_TARGET_DIR=* ]] && target=${w#CARGO_TARGET_DIR=}; done
  slots=${SOLARIS_SLOT_TARGETS:-/run/media/kaiserroman/games/solaris-work/targets}
  tree=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
  mkdir -p "$dir"
  if [ -z "$target" ]; then
    min_free=${SOLARIS_SLOT_MIN_FREE_GB:-80}
    free_gb=$(df --output=avail -BG "$slots" 2>/dev/null | tail -1 | tr -dc 0-9)
    for pass in own existing new; do
      [ "$pass" = new ] && [ "${free_gb:-0}" -lt "$min_free" ] && break
      for l in {a..z}; do
        case $pass in
          own) [ "$(slot_tree "$slots/slot-$l")" = "$tree" ] || continue ;;
          existing) [ -d "$slots/slot-$l" ] || continue ;;
          new) [ -d "$slots/slot-$l" ] && continue ;;
        esac
        exec 7>"$dir/$l" 8>>"$dir/$l.use"
        if flock -n 7 && flock -x -n 8; then picked=$l; break 2; fi
        exec 7>&- 8>&-
      done
    done
    [ -n "$picked" ] || { echo "mem-run: no free target dir (disk ${free_gb}G free, new dirs need ${min_free}G); finish or cancel another command (exit 75)" >&2; exit 75; }
    export CARGO_TARGET_DIR="$slots/slot-$picked"
    target=$CARGO_TARGET_DIR
    slot_switch_tree "$target"
    flock -s 8
    return 0
  fi
  real=$(realpath -m "$target")
  slots_real=$(realpath -m "$slots")
  case $real in
    "$slots_real"/slot-[a-z]) l=${real##*/slot-} ;;
    "$slots_real"/slot-[a-z]/*)
      # slot-gc treats it as part of the slot, but no lock would protect it.
      echo "mem-run: CARGO_TARGET_DIR $target is inside pooled ${real#"$slots_real"/}; use the slot itself or drop CARGO_TARGET_DIR (exit 75)" >&2
      exit 75
      ;;
    *) return 0 ;;  # not a pooled slot: nothing to own
  esac
  for attempt in 1 2; do
    exec 7>"$dir/$l" 8>>"$dir/$l.use"
    if flock -n 7; then
      if flock -x -n 8; then
        slot_switch_tree "$real"
        flock -s 8
        return 0
      fi
      flock -s 8  # never blocks: nobody holds it exclusively while we hold the pool lock
      [ "$(slot_tree "$real")" = "$tree" ] && return 0  # a command of this tree still runs on it
      slot_refuse "$l" "$real"
    fi
    exec 7>&-
    # Held: wait while its owner re-marks it (or slot-gc cleans it), then read its tree.
    flock -s -n 8 || { echo "mem-run: slot-$l is being re-marked or cleaned; waiting for it" >&2; flock -s 8; }
    [ "$(slot_tree "$real")" = "$tree" ] && return 0  # share with this tree's owner
    exec 8>&-  # the holder may have been slot-gc, now done: try once more to own it
  done
  slot_refuse "$l" "$real"
}
