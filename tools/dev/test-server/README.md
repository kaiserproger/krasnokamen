# Test server tools

These tools target a test environment, not production. A fresh-world reset destroys the current world; obtain explicit approval for that exact reset before running it. Never put credentials in the repository.

- `collect-metrics.py` runs on the server and aggregates closed dashboard minutes. `SOLARIS_DASHBOARD_URL` defaults to `http://localhost:25580`; `SOLARIS_METRICS_DIR` defaults to `/opt/solaris/metrics`.
- `deploy-reset.sh` runs on the server, backs up and removes `world`, swaps `solaris-v2.next` into place, and starts the service. `SOLARIS_SERVER_DIR` defaults to `/opt/solaris`, `SOLARIS_BACKUP_DIR` to `/root`, and `SOLARIS_SERVER_USER` to `solaris`. Provide a verified executable and sufficient backup space before running. This script does not upload anything.
- `solaris.service` is a sample systemd unit. Set `SOLARIS_SERVER_DIR` in `/etc/default/solaris` if needed; the default is `/opt/solaris`. Override `User` and `Group` with a systemd drop-in if the host uses a different service account.
- `soak.py <bots-binary> <clients> <report-path>` runs the bot schedule. Set `SOLARIS_TEST_HOST` to the owner-provided host (required), `SOLARIS_SOAK_SCHEDULE` to the schedule JSON path, and `SOLARIS_SOAK_AUTH` to the test-account credential; `SOLARIS_TEST_PORT` defaults to `25565`. The bot receives the credential as a command-line argument, which may be visible to other local users. Use isolated test credentials and an appropriately restricted host.
