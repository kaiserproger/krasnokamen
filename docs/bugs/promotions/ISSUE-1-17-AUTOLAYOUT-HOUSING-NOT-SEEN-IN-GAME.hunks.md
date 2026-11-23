# ISSUE-1-17-AUTOLAYOUT-HOUSING-NOT-SEEN-IN-GAME — HEAD re-break control patch

Patch: `docs/bugs/promotions/ISSUE-1-17-AUTOLAYOUT-HOUSING-NOT-SEEN-IN-GAME.patch`
(`patch:cae7712f8d8e8973…`), the row's `fix_commit`. It reverses on the clean
committed tree at `b67e1b0612` (`git apply -R --check`, plain context) and is
what a promotion passes as `--fix-patch`.

## What the fix is (origin `patch:5f50f42f…`)

The settlement planned storage, workshops, roads and lamps by itself but never
completed a new house or grew: the first Farm billed 30 dirt while the quarry
yielded 9-14, so it was never funded; the seed-short town had no free meadow
clearing; and a settlement's stored rations were spent on courtships that no
spare bed could take. The fix:

1. `farm.json` — the field's dirt border becomes air and `farmland` is the soil
   already in place (the palette's `g`/dirt entry is dropped);
2. `blueprints.rs::block_cost` — `farmland` costs nothing (and is not `dirt`);
3. `construction.rs` — a `meadow_site` admission before street work, so the
   first Farm's wheat seeds are gathered by clearing seed-bearing grass;
4. `villager/brain/social.rs` — a resident courts only while a spare bed is
   reachable, searched on the ration cadence;
5. `settlements/ui.rs` — a `meadow` label;
6. `villager/building.rs` — a meadow column is walked on while worked.

## HEAD equivalents (included)

- `farm.json` hunks 1-3: the two field rows get their `g` dirt ring and the
  palette its `"g": "dirt"` again (the asset is unchanged since the fix; the
  reverse is exact).
- `blueprints.rs` hunks 4-5: `farmland` leaves the free list and maps to `dirt`
  in the item table again.
- `construction.rs` hunk 6-7: the `meadow_site` admission removed from
  `plan_pass` and the (then un-called) `meadow_site` function deleted, exactly
  the pre-fix site pass.
- `villager/brain/social.rs` hunk 8: the spare-bed courtship guard deleted
  again (a settlement's rations are spent like vanilla's).

## Omissions

- `construction.rs`'s `gatherer_demand` rewrite — **(c)**: later commits
  rewrote it (`record.pending_order` became `access::head_order`, the catalog
  is per-settlement and `selected_bill` gates it), so the origin's pre-image has
  no HEAD equivalent.
- `settlements/ui.rs` (the `meadow` label) — **(c)**: the label match at HEAD
  has no `meadow` arm (a later commit dropped it), so there is no HEAD
  equivalent to reverse.
- `villager/building.rs` (the meadow stand rule) — **(c)**: HEAD's `in_reach`
  routes infrastructure kinds through `layout::stand`, so the origin's
  `ROAD`-only expression has no HEAD equivalent.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/ISSUE-1-17-AUTOLAYOUT-HOUSING-NOT-SEEN-IN-GAME.patch`
on the committed tree `b67e1b0612` succeeds.

Control runs (reversed tree, then restored):

- `settlements::construction::site_tests::a_town_short_of_wheat_seeds_clears_a_meadow_for_its_first_farm`
  → `.analysis/validation/20261123T201419-test-5h5f7z3b` (`status: failed`):
  `construction_site_tests.rs:1210` — `left: Some("road")`, `right: Some("meadow")`
  ("no building is funded, so the free time gathers seeds").
- `settlements::blueprints::tests::the_first_farm_costs_no_dirt`
  → `.analysis/validation/20261123T202002-test-ze6ddyre` (`status: failed`):
  `blueprints_tests.rs:186` — "farm level 1 must not bill dirt".
- `villager::brain::tests::breeding_tests::fed_residents_with_no_spare_bed_keep_their_bread`
  → `.analysis/validation/20261123T201802-test-e6ofktkf` (`status: passed`):
  the courtship hunk is a fix hunk but this ref does not witness it (the bread
  stays because the no-bed path at `free_bed`/`nodes_exhausted` still holds);
  recorded as **not evidence** for the control, and the graphical
  `harness:civilization-seed-qa:births` red is the integrated sweep's.

The first receipt was taken before the un-called `meadow_site` function was
deleted; deleting an uncalled function cannot change admission, and the second
and third receipts were taken on the final control tree (which compiles).
