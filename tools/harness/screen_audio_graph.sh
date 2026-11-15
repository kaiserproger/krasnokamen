#!/bin/sh
# Run a command inside a PRIVATE PipeWire + WirePlumber graph (own runtime dir and D-Bus).
# The graph has no hardware at all: WirePlumber's audio, Bluetooth and video-capture
# features are disabled and the script aborts if any alsa_/bluez_ node is visible, so
# nothing can reach the desktop's speakers, headset or microphone.
# Example: tools/screen_audio_graph.sh cargo test --lib computer::audio -- --ignored
#
# Concurrent invocations are independent by construction: every run gets its own
# mktemp runtime directory, its own D-Bus session, its own PipeWire/WirePlumber
# instance and therefore its own null sink, so split gate parts (one
# screens-voice-qa leg each) never share an audio sink. The directory is removed
# on exit, which is why this script must not exec its child.
set -eu
root=$(mktemp -d "${SA_SCRATCH:-/tmp}/solaris-pw.XXXXXX")
trap 'rm -rf "$root"' EXIT
mkdir -p "$root/config/wireplumber/wireplumber.conf.d" "$root/state" "$root/data" "$root/home"
cat >"$root/config/wireplumber/wireplumber.conf.d/90-no-hardware.conf" <<'CONF'
wireplumber.profiles = {
  main = {
    hardware.audio = disabled
    hardware.bluetooth = disabled
    hardware.video-capture = disabled
  }
}
CONF
export KRASNOKAMEN_PW_ROOT="$root"
# A wrapped command may nest tools/dev/mem-run.sh (a ledger control's release-server build), which reaches the
# user's systemd manager through $XDG_RUNTIME_DIR/systemd/private: link only that directory into the private runtime.
host_runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
if [ -d "$host_runtime/systemd" ]; then ln -s "$host_runtime/systemd" "$root/systemd"; fi
# Do not exec: replacing this shell drops the EXIT trap above and leaks the
# private graph directory (a run left thousands of /tmp/solaris-pw.* behind).
status=0
dbus-run-session -- sh -eu -c '
  export PIPEWIRE_REMOTE=pipewire-0 XDG_RUNTIME_DIR="$KRASNOKAMEN_PW_ROOT" PIPEWIRE_RUNTIME_DIR="$KRASNOKAMEN_PW_ROOT" \
    XDG_CONFIG_HOME="$KRASNOKAMEN_PW_ROOT/config" XDG_STATE_HOME="$KRASNOKAMEN_PW_ROOT/state" \
    XDG_DATA_HOME="$KRASNOKAMEN_PW_ROOT/data" HOME="$KRASNOKAMEN_PW_ROOT/home"
  unset DISPLAY WAYLAND_DISPLAY PIPEWIRE_NODE PULSE_SINK PULSE_SERVER PULSE_SOURCE DBUS_SYSTEM_BUS_ADDRESS
  pw= wp= pulse=
  stop() {
    status=$?
    trap - EXIT
    kill $pulse $wp $pw 2>/dev/null || :
    wait $pulse $wp $pw 2>/dev/null || :
    if [ -n "${KRASNOKAMEN_AUDIO_RECEIPT_DIR:-}" ]; then
      for log in pipewire wireplumber pulse; do
        if [ -f "$KRASNOKAMEN_PW_ROOT/$log.log" ]; then
          cp "$KRASNOKAMEN_PW_ROOT/$log.log" "$KRASNOKAMEN_AUDIO_RECEIPT_DIR/"
        fi
      done
    fi
    exit "$status"
  }
  trap stop EXIT
  pipewire >"$KRASNOKAMEN_PW_ROOT/pipewire.log" 2>&1 &
  pw=$!
  # Subscribe before checking the socket, so creation cannot race the wait.
  if ! python3 -c "import os; from pathlib import Path; from tools.harness.runtime import wait_for_path_condition; p = Path(os.environ[\"PIPEWIRE_RUNTIME_DIR\"]) / \"pipewire-0\"; wait_for_path_condition(p, 30, lambda: (p.is_socket(), None))"; then
    echo "ABORT: private PipeWire socket did not become ready; see pipewire.log" >&2
    exit 3
  fi
  if ! timeout 30 pw-dump >/dev/null; then
    echo "ABORT: private PipeWire registry is unavailable; see pipewire.log" >&2
    exit 3
  fi
  wireplumber >"$KRASNOKAMEN_PW_ROOT/wireplumber.log" 2>&1 &
  wp=$!
  # WirePlumber publishes settings only after connecting and managing the graph.
  if ! timeout 30 pw-mon 2>/dev/null | grep -m1 "sm-settings" >/dev/null; then
    echo "ABORT: private WirePlumber did not publish sm-settings; see wireplumber.log" >&2
    exit 3
  fi
  if [ -n "${KRASNOKAMEN_AUDIO_RECEIPT_DIR:-}" ]; then
    pipewire-pulse >"$KRASNOKAMEN_PW_ROOT/pulse.log" 2>&1 &
    pulse=$!
  fi
  # Pre-flight: a hardware-free graph. Abort if any ALSA or BlueZ object is visible.
  if ! dump=$(timeout 30 pw-dump); then
    echo "ABORT: private PipeWire hardware preflight could not read registry" >&2
    exit 3
  fi
  if printf "%s" "$dump" | grep -Eq "alsa_|bluez_|api\.alsa|api\.bluez"; then
    echo "ABORT: private graph can see hardware audio nodes" >&2
    printf "%s" "$dump" | grep -E "alsa_|bluez_|api\.alsa|api\.bluez" | head -5 >&2
    exit 3
  fi
  echo "preflight: no alsa_*/bluez_* objects in the private graph ($(printf "%s" "$dump" | grep -c "node.name") nodes)" >&2
  if [ -n "${KRASNOKAMEN_AUDIO_RECEIPT_DIR:-}" ]; then
    export PULSE_SERVER="unix:$KRASNOKAMEN_PW_ROOT/pulse/native" \
      PULSE_SOURCE=agents-silent-input PULSE_SINK=agents-null ALSOFT_DRIVERS=pulse \
      ALSA_CONFIG_PATH="$KRASNOKAMEN_PW_ROOT/alsa.conf"
    python3 -c "from tools.harness.audio import prepare_private_audio; prepare_private_audio()" || exit $?
  fi
  status=0
  "$@" || status=$?
  exit $status
' sh "$@" || status=$?
exit $status
