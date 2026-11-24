# TIME-LEGACY-FROZEN-CLOCK-CONVERSION: patch-mode fix identity

Original fix: `251276077` "world: explicitly convert legacy frozen clocks on
disposable copies". `fix_commit = "patch:f55fc4af759871d4d9a247106ce4e00945d114c9d55e1b7f2234b3e43d29cc13"`
names the villager-memory normalizer in
`docs/bugs/promotions/TIME-LEGACY-FROZEN-CLOCK-CONVERSION.patch`.

## Why not the commit

The commit introduced `datapacks::worldgen::open`, and code outside its file set
(`native_checkpoint/game_clock_upgrade.rs:276`, added by the same commit and
kept by later work) calls it with seven arguments; reversing the commit
therefore leaves E0425. A compile failure is not a control.

## Carried (1 hunk)

| HEAD |
|---|
| `native_checkpoint/game_clock_upgrade.rs:37-88` `normalize_villager` body |

The `a/`-side keeps the function's signature and the raw counters (the caller
compiles and the `Conversion` report is unchanged in shape) but rewrites
nothing: every held Work/sleep memory stays raw, which is exactly the
pre-conversion behaviour (class (b): kept API). No assertion text or counter is
invented: both sides use the registered tests unchanged.

## The control

`native_checkpoint::game_clock_upgrade::tests::conversion_memory_inclusive_boundaries_and_future_refusal`
fails at `game_clock_upgrade_tests.rs:39` (`left: 9`, `right: 18`) and
`core::native_checkpoint_crash_tests::persistence_drills::clock_conversion_tests::legacy_game_clock_conversion_folds_tail_anchors_active_and_drops_phantom_debt`
fails at `native_clock_conversion_drill_tests.rs:124` (`left: (0, 0)`).

`conversion_zero_game_one_tick_skips_all_memories` is green in the control by
construction (a zero game tick is skipped either way) and is a green
regression; the `mc-world-conversion` and `mc-world-tools` refs are green on
every tree and are regression coverage.
