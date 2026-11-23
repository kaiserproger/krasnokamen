#!/bin/bash
# Runs ON the VPS. Fresh-world redeploy; every destructive step depends on the previous one.
set -euo pipefail
cd "${KRASNOKAMEN_SERVER_DIR:-/opt/solaris}"
test -x krasnokamen.next
systemctl start solaris-metrics.service || true
# The tracked unit is `solaris` (solaris.service) and it runs as `krasnokamen`
# (solaris.service:User=): checking the `krasnokamen` unit or defaulting the file owner
# to `solaris` both guarded nothing (TEST-SERVER-FRESH-RESET-WRONG-UNIT-AND-OWNER).
systemctl stop solaris
if systemctl is-active --quiet solaris; then echo "solaris still active after stop" >&2; exit 1; fi
# A stopped unit is not proof the process is gone: a surviving server would keep writing
# the world we are about to delete.
if pgrep -x krasnokamen >/dev/null; then echo "a krasnokamen process survived the stop; refusing to delete the world" >&2; exit 1; fi
# Seconds and pid: a minute-resolution name can collide, and `tar czf` over an existing archive
# truncates it (a second reset inside one minute would silently replace the only backup).
backup=${KRASNOKAMEN_BACKUP_DIR:-/root}/world-backup-$(date +%Y%m%d-%H%M%S)-$$.tgz
if [ -e "$backup" ]; then echo "backup $backup already exists; refusing to overwrite" >&2; exit 1; fi
tar czf "$backup" world
# Read the whole listing before judging it: `tar tzf ... | grep -q` lets grep exit early, tar then
# dies of SIGPIPE under pipefail (a false refusal AFTER the service stop) and the full-stream read
# that also catches a corrupt tail is gone.
listing=$(tar tzf "$backup") || { echo "backup $backup cannot be listed (corrupt or truncated)" >&2; exit 1; }
grep -q '^world/' <<<"$listing" || { echo "backup $backup does not contain world/" >&2; exit 1; }
echo "backup ok: $backup $(du -h "$backup" | cut -f1)"
rm -rf world
mv krasnokamen krasnokamen.prev
mv krasnokamen.next krasnokamen
chown "${KRASNOKAMEN_SERVER_USER:-krasnokamen}:" krasnokamen
since=$(date '+%Y-%m-%d %H:%M:%S')
echo "starting"
systemctl start solaris
# --since is taken before the start so a fast "is listening" line cannot be missed.
line=$(timeout 400 journalctl -u solaris -f --since "$since" -o cat | grep -m1 -E "is listening|failed to start|panic" || true)
echo "$line"
case "$line" in *"is listening"*) echo READY ;; *) echo "START FAILED"; exit 1 ;; esac
