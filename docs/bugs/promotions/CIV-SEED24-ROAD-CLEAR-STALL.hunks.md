# CIV-SEED24-ROAD-CLEAR-STALL — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-ROAD-CLEAR-STALL.patch` (`patch:a3360a2db04ed9c2ef5d9219901abf926e00ab85dcecdd6398d2230a19f90fd1`), the row's
`fix_commit`. It reverses on the committed tree (`git apply -R --check`, plain
context) and is what a promotion passes as `--fix-patch`.

## What the fix is (origin `2b8219d1`, a commit)

Files: `crates/mc-core/src/villager/building.rs` (plus its tests and a guide
line, below).

`preparation_stand` requires a cell the navigator can end a route on
(`allocator::standing`) instead of the collision-shape `clear` test that counted
a 1.5-high fence top as ground. The earlier `fix-patches/CIV-SEED24-ROAD-CLEAR-STALL.patch`
copy was deleted: every added line of it is in commit `2b8219d1` (checked
mechanically against `git show 2b8219d1` for all four files), so the commit is the
record and the copy was redundant.

## HEAD equivalents and omissions

HEAD carries a later `lava_safe` term (D10, another row) which the re-break keeps;
only `stands`/`dry` are reverted to `clear`.

- `building_preparation_tests.rs` — **(b)**: the kept test
  `villager::building_preparation_tests::…` drives the fixed decision and is the
  row's check; reversing it would delete the control's red.
- `docs/settlements/construction.md` (one sentence, "A stand is a cell the
  builder's body can stand on, so the top of a fence or wall post never counts") —
  **(c) and not part of any patch**: promotion patches never contain `docs/` or
  `.md` hunks (`ledger._split_non_test` skips them), and the sentence is already at
  HEAD (`docs/settlements/construction.md:177`), so there is nothing to reverse.
- `docs/bugs/ledger.toml` — **(c)**: the ledger was split into `docs/bugs/rows/`
  afterwards; no HEAD equivalent.

## a/-side audit (B5, 2026-11-24)

`git show 2b8219d1^:crates/mc-core/src/villager/building.rs` vs the file after
`git apply -R`: the reversed hunk's old block (`if clear` → `if dry && stands`) is
the pre-image; the debug/test hunks are omitted as (b). No other difference.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIV-SEED24-ROAD-CLEAR-STALL.patch` on
the committed tree succeeds (clean tree); the real-client red is the integrated
sweep's control run, not claimed here.
