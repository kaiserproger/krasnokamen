# Test server tools

These tools target a test environment, not production. Resetting a world is
destructive and always requires explicit owner approval for that exact reset.
Never put credentials in the repository.

- `collect-metrics.py` runs on the server and aggregates closed dashboard minutes. `KRASNOKAMEN_DASHBOARD_URL` defaults to `http://localhost:25580`; `KRASNOKAMEN_METRICS_DIR` defaults to `/opt/solaris/metrics`.
- **Do not run the checked-in `deploy-reset.sh`.** It is destructive, not a
  world-preserving redeploy. See the [stop/check and ownership mismatch](../../../docs/development/test-server-ops.md#fresh-world-reset)
  in the owning procedure before any reset is considered.
- `solaris.service` is a sample systemd unit. Set `KRASNOKAMEN_SERVER_DIR` in `/etc/default/solaris` if needed; the default is `/opt/solaris`. Override `User` and `Group` with a systemd drop-in if the host uses a different service account.
- `soak.py <bots-binary> <clients> <report-path>` runs the bot schedule. Set `KRASNOKAMEN_TEST_HOST` to the owner-provided host (required), `KRASNOKAMEN_SOAK_SCHEDULE` to the schedule JSON path, and `KRASNOKAMEN_SOAK_AUTH` to the test-account credential; `KRASNOKAMEN_TEST_PORT` defaults to `25565`. The bot receives the credential as a command-line argument, which may be visible to other local users. Use isolated test credentials and an appropriately restricted host.
