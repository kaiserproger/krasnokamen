# CIV-UNFUNDED-SITE-SWEEP-BLOCKS-UPGRADE — forward-ported fix patch

Forward patch: `docs/bugs/promotions/CIV-UNFUNDED-SITE-SWEEP-BLOCKS-UPGRADE.patch`
(`patch:27e4358a39c30bec784213e18d1e8a0bf14517364eec97e61407d4f510ab04dd`), the
row's `fix_commit`. It applies in reverse on the candidate `wt/D123Civ`
(`git apply -R --check`) and is the net non-test diff from the base
`f9a8d45596696880fc6a0c9d92f5cd1758002651` to the fixed tree for the two
production files the many-row path touches. Test files are not in it: the
regressions live in `crates/mc-core/src/settlements/construction_growth_tests.rs`
and stay in the control worktree.

## Why patch mode (two commits)

The path was closed by two commits on `wt/D123Civ`:

| commit | what it closes |
|---|---|
| `968589107` fix(settlements): a sweep holds the slot only for the kinds the pass advanced | first cut: `Advance::Incomplete { next, holding }`, per-kind accumulation in `site_project`, the refused-materialization keep, `SitePass::house_advanced`, and the four regressions |
| `__REFACTOR__` refactor(settlements): query a sweep's hold per kind | replaces the enum reshape with `Sweeps::holding(kinds)`; the enum and `advance`'s signature stay as at base, so `construction_terrain_tests.rs` keeps its original `Incomplete(u32)` patterns |

`968589107` alone cannot be the reversal target: reversing it restores the old
production but leaves the candidate's *kept* terrain tests matching
`Advance::Incomplete { .. }`, which no longer exists → the control would not
build. The net patch has no such mismatch: reversed, both trees agree on the
tuple variant.

The A9IntegC lane's own patch
(`receipts/A9IntegC/layers/09-sweep-hold-materialize.patch`) used the enum
reshape; this port keeps its semantics (`advance` still knows its own sweep's
hold — `holding(kind)` reads the same map entry, and a sweep the recheck moved
under is removed before returning `Incomplete(0)`, so it holds nothing) without
touching the shared test surface.

## Hunks → HEAD equivalents

| hunk | HEAD (fixed) | disposition |
|---|---|---|
| `construction_site_refusals.rs`: `Sweeps::waiting()` (any live sweep holding a candidate) | replaced by `Sweeps::holding(kinds)` | **included** |
| `construction.rs`: `SitePass.waiting` doc scopes the hold to the kinds the pass advanced | `construction.rs:1497-1506` | **included** (docs) |
| `construction.rs`: `SitePass::house_advanced` field and its doc | `construction.rs:1507-1510` | **included** |
| `construction.rs`: `advanced` ids + `house_advanced` flags in the loop | `construction.rs:1550-1585` | **included** |
| `construction.rs`: a refused materialization returns `running` and the kinds' hold instead of `false`/`false` | `construction.rs:1615-1635` | **included** |
| `construction.rs`: the pass's final `waiting` is `sweeps.holding(advanced)` | `construction.rs:1635-1647` | **included** |
| `construction.rs`: `plan_pass`'s synthetic `SitePass` sets `house_advanced: false` | `construction.rs:1763` | **included** |
| `construction.rs`: `plan_pass` lets an ordinary House revisit own the hold | `construction.rs:1810-1826` | **included** |

No a/-side adaptation and no omitted production hunk: the patch is exactly the
base-to-fixed diff of these two files.

## Proving refs

- `…::a_stale_unfunded_sweep_does_not_hold_the_slot_for_a_funded_upgrade` — the
  row's proof: red at the decision assertion with the patch reversed, green on
  the restore.
- `…::a_funded_sweep_that_still_holds_a_candidate_keeps_the_slot` — the same-kind
  hold stays green in the control.
- `…::an_ordinary_revisit_of_the_priority_house_owns_the_hold` — covers
  `house_advanced`; green in both trees.
- `…::a_refused_materialization_keeps_the_earlier_hold` — covers the
  materialization keep; green in both trees.
