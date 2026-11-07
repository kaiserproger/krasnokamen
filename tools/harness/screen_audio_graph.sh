#!/bin/sh
# Run a command inside a PRIVATE PipeWire + WirePlumber graph (own runtime dir and D-Bus).
# The graph has no hardware at all: WirePlumber's audio, Bluetooth and video-capture
# features are disabled and the script aborts if any alsa_/bluez_ node is visible, so
# nothing can reach the desktop's speakers, headset or microphone.
# Example: tools/screen_audio_graph.sh cargo test --lib computer::audio -- --ignored
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
export SOLARIS_PW_ROOT="$root"
exec dbus-run-session -- sh -c '
  export XDG_RUNTIME_DIR="$SOLARIS_PW_ROOT" PIPEWIRE_RUNTIME_DIR="$SOLARIS_PW_ROOT" \
    XDG_CONFIG_HOME="$SOLARIS_PW_ROOT/config" XDG_STATE_HOME="$SOLARIS_PW_ROOT/state" \
    XDG_DATA_HOME="$SOLARIS_PW_ROOT/data" HOME="$SOLARIS_PW_ROOT/home"
  unset DISPLAY WAYLAND_DISPLAY PIPEWIRE_NODE PULSE_SINK PULSE_SERVER PULSE_SOURCE DBUS_SYSTEM_BUS_ADDRESS
  pipewire >"$SOLARIS_PW_ROOT/pipewire.log" 2>&1 &
  pw=$!
  wireplumber >"$SOLARIS_PW_ROOT/wireplumber.log" 2>&1 &
  wp=$!
  stop() { kill $wp $pw 2>/dev/null; wait $wp $pw 2>/dev/null; }
  if [ -n "${SOLARIS_AUDIO_RECEIPT_DIR:-}" ]; then
    pipewire-pulse >"$SOLARIS_PW_ROOT/pulse.log" 2>&1 &
    pulse=$!
    stop() { kill $pulse $wp $pw 2>/dev/null; wait $pulse $wp $pw 2>/dev/null; }
  fi
  trap stop EXIT
  # WirePlumber publishes its settings metadata once it manages the graph: wait for that event.
  timeout 30 pw-mon 2>/dev/null | grep -m1 "sm-settings" >/dev/null
  # Pre-flight: a hardware-free graph. Abort if any ALSA or BlueZ object is visible.
  dump=$(pw-dump)
  if printf "%s" "$dump" | grep -Eq "alsa_|bluez_|api\.alsa|api\.bluez"; then
    echo "ABORT: private graph can see hardware audio nodes" >&2
    printf "%s" "$dump" | grep -E "alsa_|bluez_|api\.alsa|api\.bluez" | head -5 >&2
    exit 3
  fi
  echo "preflight: no alsa_*/bluez_* objects in the private graph ($(printf "%s" "$dump" | grep -c "node.name") nodes)" >&2
  if [ -n "${SOLARIS_AUDIO_RECEIPT_DIR:-}" ]; then
    export PULSE_SERVER="unix:$SOLARIS_PW_ROOT/pulse/native" \
      PULSE_SOURCE=agents-silent-input PULSE_SINK=agents-null ALSOFT_DRIVERS=pulse \
      ALSA_CONFIG_PATH="$SOLARIS_PW_ROOT/alsa.conf"
    python3 -c "from tools.harness.audio import prepare_private_audio; prepare_private_audio()" || exit $?
  fi
  status=0
  "$@" || status=$?
  exit $status
' sh "$@"
