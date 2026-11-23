# VILLAGER-LEDGE-DESCENT-OVERSHOOT-LIVELOCK — forward-ported fix patch

Forward patch: `docs/bugs/promotions/VILLAGER-LEDGE-DESCENT-OVERSHOOT-LIVELOCK.patch`
(`patch:d45b693071c4ae58af15483ae4f1f61c476b4c31e739cdaff2059c43795ad2b5`).
It applies in reverse on the candidate `wt/D123Civ` at
`f9a8d45596696880fc6a0c9d92f5cd1758002651` (checked with `git apply -R --check`).

## Why patch mode

The fix commit `c5bb7877bc2ee713fab9b9c4768ec15abc07e76f` ("fix(villager): bound
walking steps by waypoint distance") no longer reverses on the candidate: later
commits rewrote the arrival and step code in
`crates/mc-core/src/villager/brain/motion.rs`. A control worktree at that commit
cannot run the registered refs either: `ledger promote` refused with

    cargo:mc-core:villager::brain::motion::tests::a_lower_committed_waypoint_does_not_reverse_horizontal_motion_while_falling=unrunnable (unknown or ignored test)
    cargo:mc-core:villager::brain::motion::tests::an_idle_resident_descends_a_ledge_at_every_start_phase_without_livelock=unrunnable (unknown or ignored test)

because the `motion::tests` module registration arrived later. Receipt:
`.analysis/validation/20261123T145502-ledger-promote-_4nc3ti3`.

## Original commit hunks → HEAD equivalent

| original hunk | HEAD equivalent | disposition |
|---|---|---|
| `motion.rs`: three lines added in `resolve_body_motion` — the two-line comment and `let speed = speed.min(length / dt);` before `horizontal = [dx / length * speed, …]` | same three lines at `crates/mc-core/src/villager/brain/motion.rs:708-710`; a later commit reworded the first comment line to "until its vertical arrival" | **included** in the patch |
| `motion.rs`: `#[cfg(test)] #[path = "motion_tests.rs"] mod tests;` appended at the end of the file | present at HEAD | **omitted (b)**: API the kept tests call — reversing it stops the control from compiling, so the control could never reach an assertion |
| `motion_tests.rs`: new 152-line test file | present | kept (test file; test hunks are never reversed) |
| `docs/architecture/settlements.md`: six lines describing the bound | present | kept (docs) |

No a/-side adaptation was needed: the patch is exactly the three guarding lines.

## Proving ref

`cargo:mc-core:villager::brain::motion::tests::a_lower_committed_waypoint_does_not_reverse_horizontal_motion_while_falling`
— reversing the included hunk makes the body step past the waypoint's horizontal
centre while falling, which flips the yaw the assertion holds fixed
(`motion_tests.rs`, the "no yaw flip" assertion). The second registered ref
(`an_idle_resident_descends_a_ledge_at_every_start_phase_without_livelock`) is
red for the same reversal.
