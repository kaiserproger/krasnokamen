# CIV-SEED24-STALLED-PROJECT-HOLDS-SLOT — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-STALLED-PROJECT-HOLDS-SLOT.patch`
(`patch:c289957a9d229a91…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `671b5c41`, CIV-SEED24-STALLED-PROJECT-HOLDS-SLOT)

`assign_builder` starts each planning pass with `release_stalled_builder`,
which watches the active project's work counter (3000 shift ticks without a
cell: a builder that cannot advance), releases the incumbent and remembers it
as shunned for 24,000 ticks, and the candidate loop ranks a shunned resident
ten places lower. Seed 24: a stationary zombie eight blocks from the only road
kept the Lumber camp builder fleeing and returning for 15,000 ticks at 35 of
239 cells while the funded House waited for the single slot.

## HEAD equivalent: the two behaviour hunks

`settlements::construction` has since been split; the watch now lives in
`construction.rs` (`BuilderWatch`, `watch_project`), is keyed by
`(SettlementId, handle)` and also reports D8 stall recoveries. The row's
behaviour at HEAD is exactly two hunks of `construction_builders.rs`:

1. the `release_stalled_builder(state, id)` call at the head of
   `assign_builder` (removed by the control: no pass ever runs the watch, so a
   stalled builder is never replaced);
2. the shunned-priority block in the candidate loop (removed by the control:
   a resident the watch had shunned is not chosen last).

## Omissions

- `settlements/construction.rs` (constants `BUILDER_STALL_SHIFT_TICKS`/
  `BUILDER_SHUN_TICKS`/`BUILDER_WATCH_STEP`, `BuilderWatch`,
  `release_stalled_builder`, `watch_project`) — **(b)**: the kept regression
  `construction_site_tests.rs` names `BUILDER_STALL_SHIFT_TICKS` and drives the
  watch through `assign_builder`, and `construction_preparation.rs`
  (`tell_provenance_wait`) plus `note_build_refusal` read the same watch;
  deleting the type or constants stops the control from compiling.
- `settlements/registry.rs` (`builder_watch` field) — **(b)**: read by
  `watch_project`, `note_build_refusal` and
  `construction_preparation::tell_provenance_wait`, which stay at HEAD.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-SEED24-STALLED-PROJECT-HOLDS-SLOT.patch`
on the committed tree `b67e1b0612` succeeds.

Control run (the patch applied in reverse to the working tree, the tree then
restored): `python3 -m tools.harness run test -p mc-core --lib
settlements::construction::site_tests::builder_that_cannot_advance` →
`.analysis/validation/20261123T194330-test-sqb657tz` (`status: failed`,
`dirty` tree at head `b67e1b0612`). The red is the row's own assertion
`crates/mc-core/src/settlements/construction_site_tests.rs:440`
(`assert_eq!(profession(&state, stuck), Profession::None)`, left `Builder`):
the stale builder keeps the slot after the stall window. The graphical
`harness:civilization-seed-qa:new_house` red is the integrated sweep's control
run, not claimed here.
