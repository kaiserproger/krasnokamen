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

## Promotion withheld: the scene does not red the control deterministically

The scene's staged SIGKILL race is probabilistic: my control run went red
(`041726-battle-crash-handoff-0jkq0anw`, `crash_handoff_preserved_staged`, `handoff: null`), but the
ledger's own control run of the same reversed tree stayed green, so the harness ref is a witness, not
a control. The python ref flips deterministically — it requires the runner to recognise a SIGKILLed
server whose `poll()` is still `None`, which a reversed tree cannot provide — so it is the proving ref
and the harness ref stays green-on-candidate only.

## Missing proof (fixed-unpromoted, 2026-11-25)

`ledger promote` refuses while the user-facing scene cannot red a reversed tree: `proving_refs` may not
keep the graphical ref out (the scene carries the symptom), and the scene's staged SIGKILL race is not
deterministic — my control run red (`041726-battle-crash-handoff-0jkq0anw`), the ledger's control run of
the same reversed tree was green. The python ref does flip (the contract test requires the recognition
path, and a reversed tree fails `assertIsNotNone` at line 742), so the fix is proven at the runner level
and unproven at the scene level. The needed fixture: stage the SIGKILL *before* the console `start`, so
`mark_alive` never flips the staged startup and the reversed tree's crash watcher deterministically
reports `handoff: null` on every run.
