# SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES — patch map

Patch: `docs/bugs/promotions/SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES.patch`,
sha256 `7b9ddbcbb6e17f4921956268a3e18abb04ee7b30b109ee2a3c4a75cae1d1f932`.
Source delta: `git diff 9d1da7f8306e 44aa3d95a -- crates/mc-core/src/settlements/autonomy.rs`
(64 lines). It reverse-applies on main `277910e34` (`git apply -R --check` passes), so the
control tree is main minus exactly these hunks.

## Why patch mode and not the commit reversal

Reversing `44aa3d95a` alone does **not** restore the row's symptom. Its parent `bcd7ac471`
already carried the workaround (`fn passes_per_tick`: on a gate tick every pending village
steps, otherwise `min(VILLAGES_PER_TICK)`), and `44aa3d95a` replaced that workaround with the
per-village clock. A plain reversal therefore rebuilds the workaround, where every village does
take a gate turn, and the row's control cannot go red there. The measured starvation preimage is
`9d1da7f8306e` (`bcd7ac471^`), which has neither the per-village clock nor the workaround; the
patch carries the cumulative delta so the reversed tree equals that preimage for `autonomy.rs`.

## Commits carried, and what each closes

| commit | what it closes in the row's path |
|---|---|
| `bcd7ac471` "every pending autonomy village turns on a gate tick" | nothing this patch adds — its workaround is *not* in either side of this delta (it appears and is removed between the two endpoints), so the patch neither grants nor reverses it |
| `44aa3d95a` "a per-village gate clock instead of stepping every pending village on a gate tick" | the whole fix: `PendingVillage::next_gate`, `gate_due`, and `advance` reading that clock instead of `state.tick.is_multiple_of(20)` |

## Hunks (all six, none omitted)

1. `autonomy.rs:52` — `PendingVillage::next_gate` field plus its doc comment naming this row.
2. `autonomy.rs:162` — `next_gate: 0` at queue construction.
3. `autonomy.rs:192` — `fn gate_due(next_gate, tick)`: the village's own 20-tick clock.
4. `autonomy.rs:417` — `advance` asks `gate_due` instead of `state.tick.is_multiple_of(20)`.
5. `autonomy.rs:433` — the zone-probe condition uses the same `gate` value, so the probe and
   `prove_natural` cannot disagree about the turn.

No omitted original hunk: the delta is the fix's own non-test hunk set, and no other commit in
the range touched `autonomy.rs`. No a/-side adaptation is needed — every `-` line of the patch
is the real `9d1da7f8306e` text.

## Proving refs and the controls observed

- `cargo:mc-core:settlements::autonomy::tests::every_pending_village_gate_steps_within_the_rotation_window`
- `cargo:mc-core:settlements::autonomy::tests::every_pending_village_takes_its_gate_clock_turn`
  (rewritten to drive production `autonomy::tick` for queue lengths 1..=16; it no longer calls the
  fix-introduced `gate_due`, so the reversed tree compiles — the earlier reversal could not build.)

Red on `9d1da7f8306e` with the current test module appended, tree `wt/SpainF3-preimage`,
receipt `wt/SpainF3-preimage/.analysis/receipts/autonomy-tests-preimage-9d1da7f8306e.log`
(`FAILED. 47 passed; 2 failed; 2 ignored`):

- `autonomy_tests.rs:2643` — "every pending village must take a gate turn inside the rotation
  window, but (chunk, zone_cursor) [((100, 104), 4), ((104, 104), 4), ((108, 104), 4),
  ((112, 104), 4)] never got past the ungated first probe pass; the batch bound held at 4
  villages a tick" — the row's own symptom.
- `autonomy_tests.rs:2440` — "queue of 5 pending villages starves some village's 20-tick gate:
  [((116, 100), 4)]".

Green on `wt/SpainF3` `2b7136610` and on main after the merge
(`wt/SpainF3/.analysis/receipts/autonomy-tests-main-final.log`: `ok. 49 passed; 0 failed; 2 ignored`).
fmt and code-health green on that tree (`.analysis/validation/20261128T113200-fmt-8g6zdu_6`,
`.analysis/validation/20261128T113223-code-health-i3l3tyd1`).
