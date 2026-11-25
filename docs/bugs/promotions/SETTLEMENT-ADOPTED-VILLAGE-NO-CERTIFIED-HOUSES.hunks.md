# SETTLEMENT-ADOPTED-VILLAGE-NO-CERTIFIED-HOUSES — patch-mode map

`patch:3fc3d355930ec6cf9835531e8a1e836859f7748b5dcea5afb68932dc5150ece9` = the sha256 of this
patch's non-test hunks (one file: `crates/mc-core/src/settlements/buildings.rs`, reverses on the candidate).

`fix_commit = patch:<digest>` over the **non-test hunks of
`crates/mc-core/src/settlements/buildings.rs`** between `5060bc323f19^` and the review commit on
`wt/D123Civ`. The patch is `docs/bugs/promotions/SETTLEMENT-ADOPTED-VILLAGE-NO-CERTIFIED-HOUSES.patch`.

Why patch mode: the row's behaviour was closed by more than one commit (the review asks for
`5e073d9f5c`, `cc1d848f6b`, `5060bc323f19`), and `fix_commit` holds a single digest. `5e073d9f5c` and
`cc1d848f6b` are the classification half (beds kept, the House+Workshop pair); they are already in the
tree and already have their own recorded manual reversal, so this patch carries only the assessment half.

| commit | what it closes | buildings.rs hunks |
|---|---|---|
| `5060bc323f19` fix(settlements): an adopted generated piece certifies by function and substance | the row's cause: `assess`'s all-or-nothing envelope over the whole piece box refused every generated House | `GENERATED_STRUCTURE_CELLS`, `generated_requirements`, `assess_generated`, `inspection_points`, `inspect_generated`, `Caller` and the `certify_impl` wiring |
| review commit fix(settlements): an adopted House needs a roofed bed | ReviewD123Civ13-Opus finding: the substance floor counted any solid cell | the `allocator::roofed` condition on the bed count in `assess_generated` |

Omitted hunks, with the README reasons:

- `crates/mc-core/src/settlements/autonomy.rs` (removal of the paced refusal retry) — reason (a): it changes
  no verdict this row asserts; the refs that flip are buildings.rs behaviour. Reversing it would only
  restore a retry the run measured as useless.
- `crates/mc-core/src/core/village_generation_tests.rs`, `crates/mc-core/src/settlements/*_tests.rs` — test
  files stay (README rule 4).
- `docs/**`, `tools/harness/**`, `docs/bugs/**` — documentation, harness verdict registry and the row
  itself: not part of the control.

Proving entities (each is RED on the reversed tree at a behavioural assertion; the helper returns `Option`
so none fails inside a panic):

- `cargo:mc-core:settlements::autonomy::tests::generated_certification_tests::a_real_plains_small_house_box_certifies_as_a_house`
- `…::a_real_plains_medium_house_box_certifies_as_a_house`
- `…::a_piece_that_holds_its_own_workstation_certifies_as_the_pair`
- `…::a_lone_bed_and_door_do_not_make_a_house`
- `…::a_real_house_reduced_to_its_furniture_refuses`
- `…::a_floor_only_ruin_does_not_certify`
- `…::a_stone_floor_with_an_open_bed_does_not_certify`
- `…::an_air_box_certifies_nothing`
- `…::the_structure_floor_is_the_boundary`

Green on the control by design, therefore not regression refs of this revision (they assert the strict
path, which the reversal restores): `the_builder_envelope_still_refuses_a_real_house_box`,
`the_strict_callers_still_refuse_the_real_house_box`, and the four classification tests
(`an_adopted_house_piece_that_holds_a_workstation_certifies_both`,
`the_pair_rule_is_only_for_the_generated_caller`,
`piece_kinds_separates_the_house_the_workstation_and_neither`,
`a_piece_over_an_unreadable_column_is_classified_later_not_never`).

Graphical ref: `harness:civilization-seed-qa:adopted_houses_certified` (the gate records it at claim). It
needs a *passed* civilization-seed-qa receipt; the gate currently fails on the separate stall/funding
defect, so the row stays open.
