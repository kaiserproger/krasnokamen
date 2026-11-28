# D154 — the refreshed patches, audited against their Git pre-images

Owner D154, 2026-11-28 18:05. Source: `0bf88beb5` (PatchRefresh) merged as `e0b771447`.
The source commit changed 21 patch files: 20 additions and one modification.
For each original patch, check its old-side symbols against `git show <fix>^1:<path>`.
A fallback or helper invented on the old side is not a control. If the fix added
calls to a helper, reversing those calls without replacing the helper restores
the original dispatch behavior.

## Result: 8 of 21 are not controls, and all 8 are `open`

| row | fix | first symbol the pre-image lacks | note |
|---|---|---|---|
| `FALL-001` | `35308579c9` | `crates/mc-core/src/vanilla_interact.rs` — file absent at `^1` | the patch makes `dragon_egg` return false; the fix `35308579` is what first added the two calls `crate::vanilla_interact::dragon_egg(...)` in `crates/mc-core/src/tick/input/interact.rs` (≈84, ≈177). Correct control: keep the helper, remove the two calls, test red, restore |
| `PLACE-001` | `35308579c9` | `crates/mc-core/src/spawn_protection.rs` — file absent at `^1` | same invented-preimage shape |
| `PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL` | `2941bc91f` | `tools/coreperf/profiles.py`: `carries` | the fix adds `client_tick_arrival` outright; the minus side returns a stub whose status is unavailable and its red is a `KeyError` |
| `CIV-IRON-CAPTURE-OUTPUT-COLLECTED` | `d7f98cd4` | `tools/harness/backends/civilization_seed_qa.py`: `output_slot` | invented minus side |
| `HARNESS-REGRESSIONS-CLIENT-REBUILDS` | `a39164dc9` | `tools/harness/regressions.py`: `usable` | invented fallback that never existed |
| `HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION` | `0857a48c5` | `tools/harness/backends/screens_voice_qa.py`: `even` | invented minus side |
| `SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES` | `44aa3d95a` | `crates/mc-core/src/settlements/autonomy.rs`: `rotation` | the clock-only reversal also failed the batch bound, not the row's own progress assertion |
| `TEST-NATIVE-HEALTH` | `ef67d139e` | `tools/harness/backends/native_matrix.py`: `isinstance` | the patch invents a weaker `HealthInterval` that never existed |

Each row keeps its real `fix_commit`, stays `open`, and carries the finding in its own
text. Withdrawn in `48f50c842` (seven patches) and `d35ddfee7` (the perf **patch** — the
earlier wording "withdrew the receipt" was wrong).

## The 13 without missing old-side code

`BATTLE-DISMOUNTED-RIDER-SKATES-TO-SWEEP-POINT`, `HARNESS-CIV-GATE-BIRTH-CAPTURE-HITS-THE-WALL`,
`HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED`, `HARNESS-CIV-GATE-OBSERVER-WAIT-HANGS`,
`HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG`, `HARNESS-CIV-GATE-REMOTE-SCAN-UNLOADED`,
`HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT`, `HARNESS-EXCLUSIVE-WINDOW-SCENE-SLOT-DEADLOCK`,
`HARNESS-JAVA-MATRIX-26-1-2-EVENTS-NONE-ADAPTER-TYPEERROR`,
`HARNESS-JAVA-MATRIX-FOREIGN-PORT-AUTH-ACK`, `SETTLEMENT-HOUSE-BILL-STALLS-BELOW-COBBLESTONE-DEMAND`,
`VILLAGER-BABY-WADES-INTO-HEAD-DEEP-WATER`, `DIMENSION-SWITCH-KNOCKBACK-DISCONNECT`.

An independent comparison of the original patch blobs, trimming indentation and
ignoring comments, found their removed code lines in `git show <fix>^1:<path>`.
This includes the modified dimension-switch patch (`5810902c7^1`,
`crates/mc-edge/src/server/pump.rs`). A textual match is not a red/green run
and does not make a row promotable on its own.

## FALL-001 rebuild

Delegated back to the `PatchRefresh` lane with the owner's exact shape: the helper
`vanilla_interact::dragon_egg` stays, the two call sites in
`crates/mc-core/src/tick/input/interact.rs` are removed, the row's two `interact_parity` refs
go red at their own assertions, then green on restore. The row stays open until that
patch exists and reverses cleanly.

## Method note

A line-level comparison (`the removed line must exist verbatim in the preimage`) over-flags:
a legitimate pre-image line can differ by whitespace or by a later refactor. The identifier
test is the one that matches the owner's rule, and it is what this table uses. Both tests
agreed on all eight rejections.

## D154b correction (owner, 18:10)

The owner sharpened the criterion: a symbol on the minus side can exist and still not
be the pre-image — `FALL-001`'s minus side *replaces* the real `dragon_egg` call with a
stub `false`, so `grep <symbol>` passes while the control is void. The authoritative test
is byte-exact: each hunk's minus side must equal the corresponding pre-image lines; one
differing line makes the patch no control and the row stays `open`.

Status of that stricter pass:
- The 8 rejections above stand: for each, the minus side names code the pre-image does
  not contain *at all* (`FALL-001`, `PLACE-001` name files absent at `^1`;
  `PERF-RECEIPT` and `TEST-NATIVE-HEALTH` name functions the fix created;
  `CIV-IRON`, `HARNESS-REGRESSIONS-CLIENT-REBUILDS`,
  `HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION` and
  `SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES` name invented identifiers).
- The byte-exact comparison of the remaining 13 is running in the `PatchRefresh` lane on
  the real trees (`git apply -R`, then compare the touched regions with
  `git show <fix>^1:<path>`). My own textual screen is not authoritative: a hunk's minus
  lines can be split by context lines, and context comes from HEAD, so a naive
  contiguous-block match over-flags. Any row that pass turns red gets withdrawn the same
  way and listed here.
- `FALL-001`'s red set is narrower than I first wrote: reversing the calls in
  `crates/mc-core/src/tick/input/interact.rs` must red
  `interact_parity_dragon_egg_use_teleports` (`crates/mc-core/src/interact_parity_tests.rs:75`);
  `interact_parity_dragon_egg_conserves_block_before_falling` (line 590) calls the helper
  directly at line 593, so a green there is not a control failure.
