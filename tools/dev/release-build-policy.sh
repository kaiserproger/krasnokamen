#!/usr/bin/env bash
# Release-build policy for the owner host (owner, 2026-11-15): one release binary
# per integrated commit, built by tools/dev/pin-release and reused by its tree
# fingerprint. A gate or a measurement takes the pinned binary; an agent never
# builds release in its branch; Main pins each integration advance.
#
# `mem-run.sh` sources this file and calls `release_build_guard "$@"` before it
# takes a slot or a build token, so a refused build costs nothing. A release build
# passes only from pin-release (it exports KRASNOKAMEN_PIN_RELEASE), from the
# harness/ledger tooling (real-client gates and ledger promote's own candidate and
# control trees, each pinned to its own tree) or with Main's explicit
# KRASNOKAMEN_RELEASE_BUILD_ALLOW=1. Tests: tools/dev/test_mem_run_release_policy.py.

# The command words after a leading `env` (its -u NAME / -flag / VAR=value words).
release_command_after_env() {
  local -a words=("$@")
  local i=0
  if [ "${words[0]:-}" = env ]; then
    i=1
    while [ "$i" -lt "${#words[@]}" ]; do
      local w=${words[$i]}
      if [ "$w" = -u ]; then i=$((i + 2))
      elif [[ $w == *=* || ( $w == -* && $w != -- ) ]]; then i=$((i + 1))
      else break; fi
    done
  fi
  printf '%s\n' "${words[@]:i}"
}

# Whether the command is `cargo build|test|run --release` or `--profile release*`.
release_build_request() {
  local -a words=() rest=()
  mapfile -t words < <(release_command_after_env "$@")
  local i=0 found=0
  while [ "$i" -lt "${#words[@]}" ]; do
    case "${words[$i]}" in
      */cargo|cargo) found=1; i=$((i + 1)); break;;
    esac
    i=$((i + 1))
  done
  [ "$found" = 1 ] || return 1
  while [ "$i" -lt "${#words[@]}" ]; do
    case "${words[$i]}" in
      +*) ;;  # `cargo +toolchain build`
      *) rest+=("${words[$i]}");;
    esac
    i=$((i + 1))
  done
  case "${rest[0]:-}" in build|test|run) ;; *) return 1;; esac
  local j=1
  while [ "$j" -lt "${#rest[@]}" ]; do
    case "${rest[$j]}" in
      --release) return 0;;
      --profile) case "${rest[$((j + 1))]:-}" in release*) return 0;; esac; j=$((j + 1));;
      --profile=release*) return 0;;
    esac
    j=$((j + 1))
  done
  return 1
}

# pin-release and the harness/ledger tooling own release binaries.
release_tooling_caller() {
  local p=${PPID:-0} cmd
  while [ "${p:-0}" -gt 1 ]; do
    cmd=$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null || true)
    case "$cmd" in
      *tools.harness*|*tools/harness*|*pin-release*) return 0;;
    esac
    p=$(awk '$1 == "PPid:" {print $2}' "/proc/$p/status" 2>/dev/null) || p=0
  done
  return 1
}

# Refuse a release build outside pin-release (exit 75); return when the command is
# not a release build or the caller may build one.
release_build_guard() {
  release_build_request "$@" || return 0
  [ -n "${KRASNOKAMEN_PIN_RELEASE:-}" ] && return 0
  [ -n "${KRASNOKAMEN_RELEASE_BUILD_ALLOW:-}" ] && return 0
  release_tooling_caller && return 0
  cat >&2 <<'EOF'
mem-run: refusing a release build outside tools/dev/pin-release.
A release binary belongs to one integrated commit: pin it with
  tools/dev/pin-release <tree>            (--client for the client, --bots for player-scale-bots)
and pass the printed KRASNOKAMEN_*_BINARY to the gate or measurement. Re-pinning
the same tree reuses its pin instead of rebuilding; a different tree (a ledger
control, a branch) gets its own pin. Agents never build release in their branches;
Main pins each integration advance. Main may override one run with
KRASNOKAMEN_RELEASE_BUILD_ALLOW=1. Debug builds are unchanged.
EOF
  exit 75
}
