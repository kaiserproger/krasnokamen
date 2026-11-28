# wt/SpainInteg disposition (D142/D143)

Branch `wt/SpainInteg` `99b3bf0154da`, base `d9c3a591`, 22 non-merge commits ahead; main `206fe0626` is 2123 commits ahead in total.

| commit | subject | added symbols present in main |
|---|---|---|
| `a2ec37ac2` | Observe settlement planner and courtship decisions | BedCounts=NO, BedSearch=NO, Courtship=NO, CourtshipCounts=NO, ENABLED=yes, Output=yes, Plan=yes, Site=yes |
| `0cd64cdbe` | fix(settlements): rotate builders across the shun and read the watch once | Bench=yes, a_builder_shun_lifts_on_its_game_tick_deadline=NO, a_frozen_simulation_does_not_lift_a_builder_shun=NO, a_sole_shunned_native_builder_is_still_assigned=NO, an_unshunned_retrain_candidate_is_bound_before_a_shunned_native_builder=NO, assert_reaches_storage=NO, bench=yes, bound=yes |
| `6fb805771` | test(settlements): cover furnace fuel shortfall and unreadable bills | smelting_larger_masonry_bill_stalls_at_the_unplaced_torch_reservation=NO, smelting_returns_stone_supplies_and_waits_when_the_active_bill_is_unreadable=NO |
| `e1d84f9c5` | fix(settlements): count the builder stall window in simulated ticks | SHUNNED_PRIORITY=NO, a_frozen_simulation_adds_no_builder_stall_window=NO, a_held_day_clock_does_not_stop_the_builder_stall_window=NO, a_released_builder_is_not_rebound_before_an_unshunned_builder=NO, held_tick=yes, shun=yes, shunned=yes |
| `ed0697168` | fix(settlements): reserve bill fuel by quantity | FuelBudget=NO, FuelInputs=NO, fuel_budget=NO, furnace_fuel_surplus=NO, may_fuel=NO, reserve_fuel_inputs=NO, reserve_recipe_inputs=NO, smelting_keeps_iron_running_when_an_active_bill_is_unreadable=NO |
| `1fd4bc9f6` | fix(settlements): protect furnace fuel and supply derived masonry | SMELTING=NO, StoneRecipe=NO, free_build_stock=NO, fuel_protection=NO, fuel_source=NO, furnace_fuel_protection=NO, may_fuel=NO, smelting_iron_and_stone_race_for_one_furnace_without_losing_custody=NO |
| `2e2213ce7` | fix(settlements): smelt stone for active construction bills | Budget=yes, CHEST=yes, FURNACE=yes, Product=yes, SavedTask=NO, SmeltingRecipe=yes, TENDED=NO, accepts_batch=NO |
| `d8f9990f6` | test(harness): budget the retained gate by day phase and census only the shift | (no new symbols) |
| `696379ab6` | fix(harness): finalize failed growth runs without block or census frames | (no new symbols) |
| `d46fe2af6` | test(settlements): commit causal growth observer and retained stall gate | (no new symbols) |
| `33554901a` | fix(settlements): align released-carry routing with the custody fence | a_smaller_duty_caps_the_handoff_before_the_remaining_haul=NO, bank_crafting_is_preferred_to_a_released_resident_pickup=NO, donor=yes, donor_arrival_and_handoff_share_the_exact_body_distance=NO, handoff_cut=NO, in_reach=yes, near_bodies=NO, released_construction_handoff_replays_both_carries_and_the_return_duty=NO |
| `18ea1a5c3` | fix(settlements): keep released construction carry available in full villages | a_return_duty_reserves_only_its_addressable_physical_stock=NO, accounted=yes, advance_one=NO, available=yes, deduct=NO, fetch=yes, item=yes, physical_handoff_is_bounded_by_the_duty_and_preserves_stack_components=NO |
| `720176338` | fix(settlements): count both integrated site refusal reasons | REASONS=yes |
| `a0ae2b282` | fix(settlements): advance founding survey once per site | founding_split_counts_the_adult_staying_in_another_dimension=NO, founding_split_hungry_pair_without_bread_cannot_change_civil_state=NO, founding_split_skips_wandering_builders_and_outbound_walkers=NO, founding_split_surveys_each_site_once_with_multiple_households=NO, mark_outbound=NO |
| `98eab23e3` | feat(settlements): split large households for daughter founding | AFTER_SPLIT=NO, CrashChild=yes, civil_image=yes, drop=yes, family=yes, flat_site=yes, founding_split_admission_preserves_parent_children_beds_and_every_item=NO, founding_split_refused_site_has_no_household_or_membership_effect=NO |
| `2195981eb` | fix(settlements): isolate founding survey and seed apron access | birch_stock=NO, forested_corridor=NO, forestless_outpost_requires_a_route=NO, house_project=NO, one_blocked_apron_column_after_admission_keeps_other_sites_startable=NO, outpost_corridor_trees_restart_credits_each_log_once=NO, player_touched_corridor_log_is_avoided_and_preserved=NO, recover_corridor=NO |
| `ece50bfc5` | style: rustfmt construction_storage debug call | (no new symbols) |
| `7e9ad6c24` | fix(settlements): survey and prepare independent founding outposts | CLAIM_SIZE=NO, COLUMNS=yes, Node=yes, SITE_STEP=NO, STEP_DOWN=yes, STEP_UP=yes, WIDTH=yes, forested_outpost_uses_its_own_access_and_conserves_felled_logs=NO |
| `702bb6e41` | fix(villager): a perimeter goal the router cannot walk to is no goal | ROUTE_NODES=NO, STAND_ROUTES=NO, WALK_NODES=NO, a_builder_does_not_wait_beneath_a_neighbours_roof_stand=NO, walks=yes |
| `83e97e4e8` | fix(settlements): D7 review: admit terrain work by the executor's own rules, check live cuts, bill real work | Denied=yes, UNITS_PER_LOT=NO, Why=yes, a_cut_under_a_late_falling_block_is_refused_without_debit=NO, a_cut_under_the_builders_feet_is_refused_without_debit=NO, a_fill_is_served_only_from_above=NO, a_fluid_source_that_needs_the_ramp_is_a_named_refusal=NO, a_ramp_step_outside_the_territory_is_refused=NO |
| `ccc408d56` | feat(settlements): terrain ramps for deep fills and high cuts inside the territory | BODY=yes, HEAD=yes, Kind=yes, Lot=yes, MAX_BLUEPRINT_RESERVED_HEIGHT=NO, Order=yes, REACH=yes, RING=yes |
| `d5123e2e1` | feat(settlements): inside the territory terrain is built, not refused: deep fill and hillside cut by one reach rule (owner D7/D45) | ARRIVAL=yes, REASONS=yes, RING=yes, TERRAIN_REACH=NO, VILLAGER=yes, a_fill_thirty_blocks_deep_is_paid_from_storage_and_completed=NO, a_fill_twelve_blocks_deep_is_paid_from_storage_and_completed=NO, a_hillside_far_above_the_roof_is_cut_into_storage_and_the_building_completes=NO |

## Decision (D142/D143): hold the branch, reject the wholesale merge

`git merge wt/SpainInteg` onto `main 206fe0626` auto-merged nine files and left
15 conflict hunks in five code files (`settlements/construction.rs`,
`construction_preparation.rs`, `construction_stall_tests.rs`,
`construction_terrain_tests.rs`, `forestry.rs`) plus three docs. It is not a
mechanical resolution: `forestry::lot_has_tree` alone takes a different
signature because both sides restructured the construction/preparation API, and
main has since cut `mc-chunk-loader`, `mc-survival`, `mc-domain`, `mc-inventory`
and `mc-placement` out of `mc-core` (173 settlements commits since the base
`d9c3a591`). The merge was abandoned (`git merge --abort`); no main byte changed.

The branch is **not** content-stale: the table above shows nearly every added
symbol missing from main. It is architecturally stale. Decision: keep
`wt/SpainInteg` as the source patch series, and re-port it feature by feature onto
current main, each with its own focused gate, instead of one 9.8k-line merge.

Feature families to re-port, in this order (each is its own slice and ledger
entry when it starts):

1. **D7 territory terrain** — `d5123e2e1`, `ccc408d56`, `83e97e4e8` (one reach
   rule builds deep fills and hillside cuts inside the territory instead of
   refusing them; executor rules admit the work, live cuts checked, real work
   billed).
2. **Founding outposts and daughter split** — `7e9ad6c24`, `2195981eb`,
   `98eab23e3`, `a0ae2b282` (independent outpost survey/preparation, seed apron,
   large-household split, survey once per site).
3. **Construction supply** — `2e2213ce7`, `1fd4bc9f6`, `ed0697168`, `6fb805771`
   (smelt stone for active bills, fuel protection and derived masonry, fuel
   reserved by quantity, unreadable-bill and shortfall tests).
4. **Builder shun and stall accounting** — `0cd64cdbe`, `e1d84f9c5` (rotate
   builders across the shun, read the watch once, count the stall window in
   simulated ticks; `SHUNNED_PRIORITY` is absent from main).
5. **Released-carry custody** — `18ea1a5c3`, `33554901a` (released construction
   carry stays available in full villages and follows the custody fence;
   `released_carry` is absent from main — `git grep released_carry main` is empty).
6. **Villager perimeter route** — `702bb6e41` (a perimeter goal the router cannot
   walk to is no goal; `ROUTE_NODES`/`WALK_NODES` absent from main), plus the
   harness budget/finalize fixes `d8f9990f6`, `696379ab6`, `d46fe2af6`.

Nothing is retired from the branch; nothing is merged from it yet. This file is
the record the owner asked for, so the branch does not hang undecided.

## Ownership (2026-11-28 15:45 host)

- F1 terrain — lane `wt/SpainD7Re` (row `SPAININTEG-F1-TERRAIN-REACH`), porting.
- F3 construction supply, F4 builder shun — queued behind F1 on the same lane.
- F2 founding outposts — lane `wt/SpainF2` (row `SPAININTEG-F2-FOUNDING-OUTPOSTS`),
  porting since 15:52; source files under edit, no commit yet.
- F5 released carry — **no owner yet**; waits for a free lane slot. Not dropped:
  its row exists and names the source commits.
- F6 villager perimeter — queued after F4.

## Closure (D142 final, 2026-11-28)

All 22 non-merge commits on `wt/SpainInteg` have a disposition: 21 are assigned
to ledger rows and one formatting-only commit will not be ported. The branch
stays intact as a patch series; it is not merged wholesale.

| family | commits | row | state |
|---|---|---|---|
| F1 territory terrain | `d5123e2e1`, `ccc408d56`, `83e97e4e8` | `SPAININTEG-F1-TERRAIN-REACH` | in port (lane `wt/SpainD7Re`) |
| F2 founding outposts and daughter split | `7e9ad6c24`, `2195981eb`, `98eab23e3`, `a0ae2b282` | `SPAININTEG-F2-FOUNDING-OUTPOSTS` | in port (lane `wt/SpainF2`) |
| F3 construction supply | `2e2213ce7`, `1fd4bc9f6`, `ed0697168`, `6fb805771` | `SPAININTEG-F3-CONSTRUCTION-SUPPLY` | queued behind F1 on `wt/SpainD7Re` |
| F4 builder shun and stall | `0cd64cdbe`, `e1d84f9c5` | `SPAININTEG-F4-BUILDER-SHUN` | queued behind F3 |
| F5 released-carry custody | `18ea1a5c3`, `33554901a` | `SPAININTEG-F5-RELEASED-CARRY` | no owner yet, waits for a lane slot |
| F6 villager perimeter route and growth gate | `702bb6e41`, `d8f9990f6`, `696379ab6`, `d46fe2af6` | `SPAININTEG-F6-PERIMETER-ROUTE` | queued after F4 |
| F7 diagnostics only | `a2ec37ac2`, `720176338` | `SPAININTEG-F7-DIAGNOSTICS` | no owner yet, ports last, behaviour unchanged |
| style only | `ece50bfc5` | none | dropped: rustfmt of a debug call whose file the port rewrites |

Counts: 26 commits in `d9c3a591..99b3bf0154da`: 22 non-merges (21 assigned
across seven family rows, one formatting-only commit dropped) and four merges.
`99b3bf015` and `94adae5eb` have no recorded merge-specific delta in
`git show --remerge-diff`. The other two have manual resolutions: `b8333768b`
combines the storehouse-capacity and executor terrain checks in
`construction_site_refusals.rs`, and `03bc5b514` chooses `receiving_slots` over
`free` in `construction_storage.rs`. Their equivalence in the ports has not yet
been established; the 22-entry table does not account for those resolutions.

The branch is a patch series and is not merged wholesale, and no family merges
until its own row carries a control and a live check. This table is the record:
each family row names its commits, and the ledger is the queue.

## Port review outcomes (2026-11-28 17:20 host)

| family | port head | review | verdict | amend lane |
|---|---|---|---|---|
| F1 territory terrain | `wt/SpainD7Re` `187775ced` | `receipts/SpainF1Review/REPORT.md` (committee-sol) | **changes** — admission checks only the deep subset while execution fills from above, memo keyed by stage index and length, the real-work debit is unbounded, B2 is synthetic, the late-lava test is unfunded | `wt/SpainF1Fix` (`FixSpainF1`) |
| F2 founding outposts | `wt/SpainF2` `04911e5570` | `receipts/SpainF2Review/REPORT.md` (committee-sol) | **changes** — production planning drops the cross-dimension household, cursor corners collide and do not survive replay, six controls are not Git pre-images, four tests pass on a default | `wt/SpainF2Fix` (`FixSpainF2`) |
| F3 construction supply | `wt/SpainD7Re` (in port) | — | — | same lane as F1 |
| F5 released carry | `wt/SpainF5` (in port) | — | — | `SpainF5` |

Both merges were rehearsed and rolled back without touching `main`:
`integsp-f1-merge` (`415ddeb4b`) and `integsp-f2-merge` (`2575e88c3`) hold the
clean merges as evidence; `wt/IntegSp` is back at the shared head. F2's merged
tree passed its own 85 founding tests (`20261128T100631-test-fozz2wzf`), so the
amend work is about the review findings, not about the merge mechanics.

## F5 review and the unassigned families (2026-11-28 17:55 host)

| family | port head | state |
|---|---|---|
| F5 released carry | `wt/SpainF5` `cefaf35e2` | ported (7 carry tests, each red with its own hunk reverted; fmt and code-health green), review `ReviewSpainF5` running |
| F3 construction supply | `wt/SpainF3` (base `af9094c23`) | lane `SpainF3` porting `2e2213ce7`, `1fd4bc9f6`, `ed0697168`, `6fb805771` |
| F4 builder shun | `wt/SpainF4` (base `af9094c23`) | lane `SpainF4` porting `0cd64cdbe`, `e1d84f9c5` |
| F6 perimeter route | `wt/SpainF6` (base `af9094c23`) | lane `SpainF6` porting `702bb6e41` plus the harness budget/finalize commits `d8f9990f6`, `696379ab6`, `d46fe2af6` |

F3 and F4 were previously queued behind F1 on `wt/SpainD7Re`; that lane is now
amending review findings, so both moved to their own worktrees rather than wait.
No family is unowned: F7 diagnostics still waits for a free slot and keeps its
row (`SPAININTEG-F7-DIAGNOSTICS`). The decision of this file is unchanged and
now executed family by family: nothing is merged wholesale and nothing is
dropped.
