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

## Measured control, 2026-11-25 (D123BattleL)

Control tree = this head + `git apply -R` of the pre-image patch, run **from the reversed tree** (the
scene executes that tree's `tools/battle/run-battle.py`; running it from the fixed tree passes with any
server binary, which is how three earlier "control" runs came out green):

- `062846-cx5cvec4` passed — the watcher thread had already released Popen's waitpid lock when the
  telemetry exception reached the handler, so the old `poll()` sampled `-SIGKILL`.
- `063126-n2gmvb2q` red: `crash_handoff_preserved_staged: handoff null, runner_exit 1`
- `063307-30ftkveb` red: the same assertion.

So the scene reds 2 of 3, not 3 of 3. A deterministic scene cannot be built without changing the fix:
holding `Popen._waitpid_lock` before the handler makes the fix's own `wait(timeout=10)` time out, and any
later hold happens after the reversed `poll()` has already sampled. What the fix needs to be promotable
is a runner change that makes the exception path observe the exit regardless of the watcher's lock
(the current fix samples through `wait`, which the lock defeats when held), or a scene that reproduces
`telemetry exporter failed` before the watcher thread starts.

Candidate side, same day: three runs on the clean-tree pin `af9e4adcfcb1` are green
(`063921-v2c57hb1` 157.5 s, `064159-ei26jqqo` 160.6 s, `064445-fc0xpsid` 164.8 s), so the pair is
3/3 green (candidate) against 2/3 red (control). Promotion stays refused until the control is 3/3.

## Deterministic fixture, 2026-11-27 (D124PackBattle)

The scene now has a third cut, `stalled`, which is deterministic for this pair without touching the
fix:

1. after `take.ready` (the runner is in its pre-stage `next_tick` wait, the crash watcher thread is
   inside `Popen.wait()` -> `waitpid` on the child), the scene `SIGSTOP`s the server;
2. it blocks on its own read of the same dashboard endpoint the exporter is blocked on
   (`/stats`, 20 s deadline; the exporter's own read deadline is 15 s), asserts
   `stalled_telemetry_read_timed_out`, then `SIGKILL`s the server.

While the child is stopped, `waitpid` cannot return: the watcher holds `Popen._waitpid_lock` for the
whole stall, and `_internal_poll` acquires it non-blocking, so a reversed tree's `poll()` samples
`None` on **every** run and the handoff is dropped; the fix's `wait(timeout=10)` starts at the 15 s
exporter deadline and sees the kill at 20 s, 5 s inside its window. Nothing here depends on the
kernel's socket-close/waitpid ordering, which is what made the plane-SIGKILL cuts 2/3.

Assertions of the cut: `stalled_telemetry_read_timed_out` (the fixture held),
`crash_handoff_preserved_stalled` (the row's symptom; the proving assertion), and
`recovered_stalled_world_opens` after the crash copy is reopened (`battle status` shows
`side1=0 side2=0 regiments=0`) and stopped cleanly. The two plain-SIGKILL cuts stay in the scene.

Promotion route: a passed candidate receipt of `battle-crash-handoff` on a pin of the candidate
tree, then `ledger promote <id> --client-receipt <result.json> --fix-patch
docs/bugs/promotions/HARNESS-BATTLE-CRASH-HANDOFF-LOST-ON-EARLY-SIGKILL.preimage.patch`. The
promote runs the python ref and the graphical ref on the reversed control tree itself
(`_run_harness` pins that tree), so the graphical red is recorded as the negative control; no
backfill is needed. State at the end of this session: scene code implemented, not yet run.
