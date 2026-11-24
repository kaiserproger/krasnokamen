# TIME-FROZEN-CATCHUP-DEBT: patch-mode fix identity

Original fix: `c4d8c0d01` "fix(core): anchor catch-up debt to held time".
`fix_commit = "patch:e6c2c048ba36a4e768075052a5b9a13705248deec317f462621d20e3b2c11916"`
names that fix's freeze gate on the candidate in
`docs/bugs/promotions/TIME-FROZEN-CATCHUP-DEBT.patch` (a diff from the pre-fix
form to the candidate; `git apply -R` removes the gate).
Re-audit:
`python3 -m tools.harness ledger promote TIME-FROZEN-CATCHUP-DEBT --fix-patch docs/bugs/promotions/TIME-FROZEN-CATCHUP-DEBT.patch --dry-run`

## Why not the commit itself

The commit's other hunks no longer reverse on the tip: the stage-0 merges
rewrote `debt.rs`, `journal/replay.rs`, `journal.rs`, `world/clocks.rs`,
`world.rs`, `world/capture.rs`, `world/residency.rs`, `tick/residency.rs`,
`tick_control.rs`, `native_checkpoint.rs`, `weather.rs` and
`level_metadata.rs`, so `git apply -R --check` fails on six files. A reversal
that does not apply is not a control.

## Carried (1 hunk, byte-identical to the original)

| original | HEAD |
|---|---|
| abstract_sim.rs -43,6 `run` returns when the clock is not running | abstract_sim.rs:45-48 |

The hunk's blob pair (`bb24f2b6b` → `6f0ce8ebd`) is exactly the one
`git show c4d8c0d01 -- crates/mc-core/src/abstract_sim.rs` produces, so the
`a/`-side differs from the real pre-image by nothing at all; the test file is
byte-identical on both sides.

## Excluded

Every hunk below still applies in reverse at this HEAD
(`git show c4d8c0d01 -- <file> | git apply -R --check`), so a missing HEAD
equivalent is not the reason to leave them out:

- (a) another behaviour of the same commit, which this row does not own:
  - `debt.rs` (`held_anchor`, the `at()` projection), `journal.rs`
    (`SimDebtImage.held_anchor`), `weather.rs`
    (`ClimateImage.sim_debt_anchored_since`), `core/recovery.rs`
    (`restore_debt_anchor`), `world/capture.rs` and `world/residency.rs`
    (held-anchor plumbing) — TIME-DEBT-HELD-ANCHOR-PERSISTENCE (open);
  - `level_metadata.rs` slot preservation —
    PERSIST-METADATA-RECAPTURE-BYTE-ORDER (promoted; its tip blob `2953184e2`
    is HEAD byte for byte).
- (c) a later commit rewrote the text, so no HEAD equivalent exists:
  `journal/replay.rs`, `journal/replay_game_clock.rs`,
  `native_checkpoint.rs`, `tick/residency.rs`, `tick_control.rs`,
  `world.rs`, `world/clocks.rs` (`git apply -R --check` fails on each).

`debt_game_clock_tests.rs` and the other `*_tests.rs` hunks are test files and
stay on both sides by construction.

The two `debt::game_clock_tests::*` refs moved to
TIME-DEBT-HELD-ANCHOR-PERSISTENCE with the (a) hunks above. Of the refs this
promotion ran (`.analysis/validation/20261124T033121-ledger-promote-fu2j91l7`),
`abstract_sim_tests::frozen_game_clock_never_spends_existing_catch_up_debt`
(`abstract_sim_tests.rs:582`) and
`native_debt_game_clock_cold_unload_multiple_holds_restart_and_reload_are_exact`
(`native_game_clock_drill_tests.rs:506`, `Overworld, checkpoint=false`, left 0
vs right 3) are the two reds;
`frozen_debt_game_clock_still_cancels_external_edits_and_item_births` and the
three dimension/drill refs were green in that control and are regression
coverage, not control evidence.

## The control

Reversed, the frozen tick runs abstract simulation and spends the 350 owed
ticks: `abstract_sim_tests::frozen_game_clock_never_spends_existing_catch_up_debt`
fails at `abstract_sim_tests.rs:582` with `left: 0`; in the same control
`native_debt_game_clock_cold_unload_multiple_holds_restart_and_reload_are_exact`
fails at `native_game_clock_drill_tests.rs:506` (`Overworld, checkpoint=false`,
left 0 vs right 3).
`frozen_debt_game_clock_still_cancels_external_edits_and_item_births` stays
green there (`cancel_external_writes` is outside this hunk) and is recorded as a
green regression, not control evidence.
