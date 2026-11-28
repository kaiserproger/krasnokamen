# SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF — the two-commit fix and its HEAD control

Row fix identity `patch:822579e1e302205de429cb44cb37b2f93b1c8c9624f75df67bfdd964c84eb88c`
(`docs/bugs/fix-patches/SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF.patch`,
2 hunks, one production file `crates/mc-core/src/settlements/construction_furnace.rs`).
The patch reverse-applies on `bbf5b71e5` (`git apply -R --check` passes).

## Why a patch, not the commit

The row's symptom has two faces, fixed by two commits of the same class:

| commit | what it closes |
|---|---|
| `9026719a07` | `furnace::plan` skips a column whose surface is more than two blocks from the storage chest's level: the certified furnace is not on another structure's roof |
| `bbfe2e0b9` (review wave2-9, "same class as 9026719a07") | the tender's `stand` stays at the furnace's level instead of taking the highest surface in the anchor's band |

Reversing `9026719a07`'s planner hunk alone stays GREEN at HEAD: `plan`'s
`stand(state, &project, at, None)` admission now rejects the platform under the
stand's own level rule, so the fixture never reaches `[3,206,1]` and the planner
guard is masked. The fix identity is therefore the HEAD equivalent of both
(D131 patch mode), not the single commit.

## Hunk map

| # | a/-side (pre-image) | b/-side (candidate) |
|---|---|---|
| 1 (`plan`) | no level clamp | `if (y - chest[1]).abs() > 2 { continue; }` + comment (from `9026719a07`) |
| 2 (`stand`) | `let y = surface_y(state, record.anchor[1], x, z)?;` | `surface_y_between(next[1]-3..=next[1]+1)` search + `.filter(|y| (y-next[1]).abs() <= 2)` fallback (`bbfe2e0b9`'s clamp, rewritten by `e043f78ca`) |

Hunk 2 is a class-(c) rewrite: `bbfe2e0b9` added an `if … return None` clamp;
`e043f78ca` replaced it with the banded search plus filter. The HEAD equivalent
restores the real pre-image line, not the removed intermediate form.

## Red and green

- RED: `.analysis/validation/20261128T090043-test-6ydslkb4` FAIL, exit 100 — the
  row's own `a_furnace_site_stays_at_the_store_level_not_on_a_platform_above_it`
  panics at its own assertion, `construction_planner_tests.rs:555`,
  `the furnace must not stand on the platform above the store`
  (`assert_ne!(project.origin, [3, 206, 1])`).
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS on the clean
  candidate.

## Still missing

`harness:civilization-seed-qa:iron_smelted` — a passed gate on this tree is the
real-client proof (no separate site-level verdict exists; Main's CIV-B1 ruling).
