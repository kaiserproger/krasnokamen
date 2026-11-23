# VILLAGER-SWIM-BOBS-ABOVE-BED-WAYPOINT — forward-ported fix patch

Forward patch: `docs/bugs/promotions/VILLAGER-SWIM-BOBS-ABOVE-BED-WAYPOINT.patch`
(`patch:424cf2f485ea2e5e0b256fb230d078cf6e23070a9b864a0ad31fb28d9f6afe62`), applies
in reverse on the candidate `wt/D123Civ` at
`f9a8d45596696880fc6a0c9d92f5cd1758002651` (`git apply -R --check`).

## Why patch mode

Commit `b8a9fe89ef86506787d840d61db06bbdbadddd58` no longer reverses on the
candidate (the arrival code and the `WAYPOINT_VERTICAL_REACH` visibility were
rewritten later), and a control worktree at that commit cannot build the
registered refs (they moved under `villager::brain::tests` afterwards).

## Original commit hunks → HEAD equivalents

| original hunk | HEAD equivalent | disposition |
|---|---|---|
| `motion.rs` `move_committed_with_route`: inline arrival check replaced by `waypoint_reached(...)` | `motion.rs:485-489`, reverted inline band 0.6 | **included** |
| `motion.rs` `move_skipped`: same replacement (0.09 horizontal, 0.6 band) | `motion.rs:550-554` | **included** |
| `motion.rs`: `WAYPOINT_VERTICAL_REACH` + `FLOATING_WAYPOINT_VERTICAL_REACH` consts, `waypoint_reached`, `body_box`, `wet`, and `immersion` rebuilt on `body_box` | same block, `motion.rs:588-651` | **included**, except the `WAYPOINT_VERTICAL_REACH` const, which stays because `villager/building.rs:399` reads `super::brain::WAYPOINT_VERTICAL_REACH` in production: reversing it changes another row's behaviour and would turn the control red on an assertion that is not this row's (reason (a), not (b)) |
| `villager/stroll_swim_tests.rs`: 223 test lines | present | kept (test) |

The a-side is the pre-image at HEAD: `immersion` builds its
`mc_physics::EntityBody` inline again, exactly as before the commit, with the
current field names — no behaviour of another row is touched.

## Proving refs

`cargo:mc-core:villager::brain::tests::stroll_swim_tests::a_swimmer_over_a_bed_waypoint_advances_before_the_bob_dips`
and the four sibling refs of the row: with the predicate reversed, the swimmer
hovers above the bed waypoint until the bob trough, which the trace assertions
reject.
