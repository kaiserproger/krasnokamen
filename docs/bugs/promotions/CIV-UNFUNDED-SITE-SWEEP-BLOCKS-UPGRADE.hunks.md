# CIV-UNFUNDED-SITE-SWEEP-BLOCKS-UPGRADE — forward-ported fix patch

Forward patch: `docs/bugs/promotions/CIV-UNFUNDED-SITE-SWEEP-BLOCKS-UPGRADE.patch`
(`patch:8b45c6627bb5593d544183417675b38ceb4b9f554903f961288eac1ffb997583`), the
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

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/settlements/construction.rs` `@@ -1497,9 +1497,15 @@ struct SitePass {` (adapted; origin `968589107`, discovered origin):
  - HEAD `/// A kind this pass advanced ran unfinished and already holds a legal` — 396e59f0f refactor(settlements): query a sweep's hold per kind instead of reshaping Advance; in origin^: no
  - HEAD `/// candidate: its cheapest lot is worth the slot. Another kind's sweep,` — 396e59f0f refactor(settlements): query a sweep's hold per kind instead of reshaping Advance; in origin^: no
  - HEAD `/// even a now-unfunded one, never blocks other work.` — 396e59f0f refactor(settlements): query a sweep's hold per kind instead of reshaping Advance; in origin^: no
- `crates/mc-core/src/settlements/construction.rs` `@@ -1533,11 +1539,14 @@ fn site_project<'a>(` (adapted; origin `968589107`, discovered origin):
  - HEAD `let mut advanced: Vec<&'a str> = Vec::with_capacity(kinds.len());` — 396e59f0f refactor(settlements): query a sweep's hold per kind instead of reshaping Advance; in origin^: no
- `crates/mc-core/src/settlements/construction.rs` `@@ -1563,6 +1572,12 @@ fn site_project<'a>(` (adapted; origin `968589107`, discovered origin):
  - HEAD `advanced.push(blueprint.id.as_str());` — 396e59f0f refactor(settlements): query a sweep's hold per kind instead of reshaping Advance; in origin^: no
- `crates/mc-core/src/settlements/construction.rs` `@@ -1598,12 +1613,15 @@ fn site_project<'a>(` (adapted; origin `968589107`, discovered origin):
  - HEAD `// kind's unfinished sweep that holds a lot keeps the slot.` — 396e59f0f refactor(settlements): query a sweep's hold per kind instead of reshaping Advance; in origin^: no
  - HEAD `waiting: sweeps.holding(advanced.iter().copied()),` — 396e59f0f refactor(settlements): query a sweep's hold per kind instead of reshaping Advance; in origin^: no
- `crates/mc-core/src/settlements/construction.rs` `@@ -1613,7 +1631,9 @@ fn site_project<'a>(` (adapted; origin `396e59f0f`, discovered origin):
  - HEAD `house_advanced,` — 968589107 fix(settlements): a sweep holds the slot only for the kinds the pass advanced; in origin^: yes
  - a/-side `waiting: sweeps.waiting(),` — 968589107 fix(settlements): a sweep holds the slot only for the kinds the pass advanced; in origin^: no
- `crates/mc-core/src/settlements/construction_site_refusals.rs` `@@ -223,9 +223,15 @@ impl Sweeps {` (over-reverted; origin `396e59f0f`, discovered origin):
  - HEAD `kinds.into_iter().any(|kind| {` — fe7e1a09e style(settlements): rustfmt the per-kind hold query; in origin^: no
  - HEAD `self.live` — fe7e1a09e style(settlements): rustfmt the per-kind hold query; in origin^: no
  - HEAD `.get(kind)` — fe7e1a09e style(settlements): rustfmt the per-kind hold query; in origin^: no
  - HEAD `.is_some_and(|sweep| sweep.best.is_some())` — fe7e1a09e style(settlements): rustfmt the per-kind hold query; in origin^: no
  - HEAD `})` — fe7e1a09e style(settlements): rustfmt the per-kind hold query; in origin^: no
  - a/-side `/// Some unfinished sweep already holds a legal candidate.` — 968589107 fix(settlements): a sweep holds the slot only for the kinds the pass advanced; in origin^: no
  - a/-side `pub(super) fn waiting(&self) -> bool {` — 968589107 fix(settlements): a sweep holds the slot only for the kinds the pass advanced; in origin^: no
  - a/-side `self.live.values().any(|sweep| sweep.best.is_some())` — 968589107 fix(settlements): a sweep holds the slot only for the kinds the pass advanced; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
