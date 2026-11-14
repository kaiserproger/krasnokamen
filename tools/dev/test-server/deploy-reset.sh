#!/bin/bash
# Runs ON the VPS. Fresh-world redeploy; every destructive step depends on the previous one.
set -euo pipefail
cd "${SOLARIS_SERVER_DIR:-/opt/solaris}"
test -x solaris-v2.next
systemctl start solaris-metrics.service || true
systemctl stop solaris
if systemctl is-active --quiet solaris; then echo "solaris still active after stop" >&2; exit 1; fi
backup=${SOLARIS_BACKUP_DIR:-/root}/world-backup-$(date +%Y%m%d-%H%M).tgz
tar czf "$backup" world
tar tzf "$backup" >/dev/null
echo "backup ok: $backup $(du -h "$backup" | cut -f1)"
rm -rf world
mv solaris-v2 solaris-v2.prev
mv solaris-v2.next solaris-v2
chown "${SOLARIS_SERVER_USER:-solaris}:" solaris-v2
since=$(date '+%Y-%m-%d %H:%M:%S')
echo "starting"
systemctl start solaris
# --since is taken before the start so a fast "is listening" line cannot be missed.
line=$(timeout 400 journalctl -u solaris -f --since "$since" -o cat | grep -m1 -E "is listening|failed to start|panic" || true)
echo "$line"
case "$line" in *"is listening"*) echo READY ;; *) echo "START FAILED"; exit 1 ;; esac
