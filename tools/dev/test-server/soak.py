#!/usr/bin/env python3
"""24h soak: N player-scale-bots against the live test server.

One run: start bots, send {"event":"run"} on "ready", send {"event":"stop"} on
"schedule_complete", wait for "final". Any "failure" (e.g. a kick) ends the soak
with exit 1 — a failure is a finding, not something to retry around.
The caller's timeout is only an emergency bound.
"""
import json, os, subprocess, sys

host = os.environ.get("SOLARIS_TEST_HOST")
if not host:
    sys.exit("SOLARIS_TEST_HOST must be set to the test server (ssh destination; any user@ part is ignored)")
host = host.rsplit("@", 1)[-1]
schedule = os.environ.get("SOLARIS_SOAK_SCHEDULE")
if not schedule:
    sys.exit("SOLARIS_SOAK_SCHEDULE must be set to the schedule JSON path")
auth = os.environ.get("SOLARIS_SOAK_AUTH")
if not auth:
    sys.exit("SOLARIS_SOAK_AUTH must be set for the test accounts")
bots, clients, report = sys.argv[1], sys.argv[2], sys.argv[3]
addr = f"{host}:{os.environ.get('SOLARIS_TEST_PORT', '25565')}"
proc = subprocess.Popen(
    [bots, "--addr", addr, "--clients", clients, "--view-distance", "4",
     "--report", report, "--name-prefix", "soak", "--respawn",
     "--auth-password", auth, "--unassisted-login", "--open-loop",
     "--schedule-file", schedule, "--reconnect"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
samples, final = 0, None
for line in proc.stdout:
    try:
        event = json.loads(line).get("event")
    except (ValueError, AttributeError):
        event = None
    if event == "sample":
        samples += 1
        if samples % 300:
            continue
    print(line.rstrip(), flush=True)
    if event == "ready":
        proc.stdin.write('{"event":"run"}\n')
        proc.stdin.flush()
    elif event == "schedule_complete":
        proc.stdin.write('{"event":"stop"}\n')
        proc.stdin.flush()
    elif event in ("final", "failure"):
        final = event
code = proc.wait()
sys.exit(0 if final == "final" and code == 0 else 1)
