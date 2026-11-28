# D154 — the refreshed patches, audited against their Git pre-images

Owner D154, 2026-11-28 18:05. Source: `0bf88beb5` (PatchRefresh) merged as `e0b771447`.
Method the owner set: for every symbol a patch's minus side removes,
`git show <fix>^1:<path> | grep -F '<symbol>'` must find it. A symbol that only ever
appears on the plus side, or a call the fix itself introduced, means the patch is not a
control.

## Result: 8 of 20 are not controls, and all 8 are back to `open`

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

## The 12 that pass

`BATTLE-DISMOUNTED-RIDER-SKATES-TO-SWEEP-POINT`, `HARNESS-CIV-GATE-BIRTH-CAPTURE-HITS-THE-WALL`,
`HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED`, `HARNESS-CIV-GATE-OBSERVER-WAIT-HANGS`,
`HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG`, `HARNESS-CIV-GATE-REMOTE-SCAN-UNLOADED`,
`HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT`, `HARNESS-EXCLUSIVE-WINDOW-SCENE-SLOT-DEADLOCK`,
`HARNESS-JAVA-MATRIX-26-1-2-EVENTS-NONE-ADAPTER-TYPEERROR`,
`HARNESS-JAVA-MATRIX-FOREIGN-PORT-AUTH-ACK`, `SETTLEMENT-HOUSE-BILL-STALLS-BELOW-COBBLESTONE-DEMAND`,
`VILLAGER-BABY-WADES-INTO-HEAD-DEEP-WATER`.

Every identifier on their minus side is present in `git show <fix>^1:<path>`. They are
promotable in form; whether each can be promoted today depends on its own receipt shape
(the civ-gate and java-matrix rows are client-surface and need their graphical run).

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
