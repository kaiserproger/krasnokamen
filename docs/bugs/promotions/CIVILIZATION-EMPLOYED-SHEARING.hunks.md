# CIVILIZATION-EMPLOYED-SHEARING — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-EMPLOYED-SHEARING.patch`
(`patch:d6b25ed9e231460b…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `898af4725`, CIVILIZATION-EMPLOYED-SHEARING)

`villager::shepherd::work` used to run only for `Shepherd`/`None` residents.
The fix lets any eligible adult provision shears (from storage or two iron
ingots) and shear after its own job, and takes only stock that is not reserved
by another job. The row's seed-24 symptom: a construction wool bill could not
use a working adult who had shears or two iron ingots.

## HEAD equivalent: the four included hunks

`shepherd.rs` is otherwise unchanged since the fix, so the patch is the exact
reverse of `work`'s profession gate (hunk 4), the reserved-stock selection and
count clamp in `acquire_tool` (hunks 2-3), and the module's doc line (hunk 1,
a comment only).

## Omissions

- `find_sheep`'s `state.world.is_active(cx, cz)` skip — **(a)**: it is
  CIVILIZATION-INACTIVE-SHEEP-TARGET's behaviour, already READY through this
  slice; reversing it would turn that row's claim and its control red.
- `eligible()` — **(b)**: called by the kept `work` and by
  `settlements::construction_availability.rs:260`; deleting it stops the
  control from compiling.
- `work`'s `Option<Option<BlockPos>>` return type — **(b)**: the kept tests
  assert it (`shepherd_tests.rs:518`, `Some(None)`), and `villager::brain.rs`
  consumes it; reverting the signature stops the control from compiling.
- `shepherd_at` (field, `work` uses, `persistence::journal_image`) — **(b)**:
  `villager/persistence_tests.rs:1191/1203` set and assert the field; removing
  it stops the control from compiling.
- `villager/brain.rs`'s after-work fallback — **(b)**: it calls
  `shepherd::work` with the `Option<Option<BlockPos>>` contract that the kept
  tests pin; the pre-fix match-arm shape needs the old signature.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIVILIZATION-EMPLOYED-SHEARING.patch`
on the committed tree `b67e1b0612` succeeds.

Control run (the three code hunks applied in reverse to the working tree, the
tree then restored; the fourth hunk is the doc comment and cannot change
behaviour): `python3 -m tools.harness run test -p mc-core --lib
villager::shepherd_tests::working_miner_crafts_shears_and_delivers_wool` →
`.analysis/validation/20261123T194851-test-o6na7egf` (`status: failed`,
`dirty` tree at head `7d6ca53c54`). The red is the row's own assertion
`crates/mc-core/src/villager/shepherd_tests.rs:355`
(`assert_eq!(wool_of(...), 0x10)`, left `0`): the employed miner never shears
the sheep. The graphical `harness:village-qa:shepherd_shears` red is the
integrated sweep's control run, not claimed here.
