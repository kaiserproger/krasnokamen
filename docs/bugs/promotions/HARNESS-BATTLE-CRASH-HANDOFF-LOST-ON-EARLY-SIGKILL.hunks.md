# HARNESS-BATTLE-CRASH-HANDOFF-LOST-ON-EARLY-SIGKILL — fix-hunk map

Fix `610f31cb0` (python only). Control: `patch:2855347b0693a86d7d07d0c99798469007a57f0f1933585c43686dc85b502f8b`
(`docs/bugs/promotions/HARNESS-BATTLE-CRASH-HANDOFF-LOST-ON-EARLY-SIGKILL.preimage.patch`), the
commit's own non-test hunk reversed verbatim (`git apply -R` applied cleanly — no hand-porting, no
stub): the `except` branch's `killed_by_sigkill(server)` goes back to `server.poll() == -signal.SIGKILL`
and the helper disappears.

| file | what it does |
|---|---|
| `tools/battle/run-battle.py` | `killed_by_sigkill()` waits for the reaped exit instead of sampling `poll()` while the crash watcher holds the waitpid lock, so a SIGKILL that closed the telemetry socket first is still recognised and the handoff runs. |

## Control

- The gate (`run scenario battle-crash-handoff --client native`, 1500 a side, SIGKILL before staging
  and staged-before-start) must be red on a tree with this patch reversed, on the staged-before-start
  cut, and green on the candidate — the runner's own unit test
  `test_crash_handoff_sees_sigkill_while_crash_watcher_holds_waitpid` is the fast half of the pair.
- The patch touches no server code: the server binary (and its pin) is identical on both sides.
