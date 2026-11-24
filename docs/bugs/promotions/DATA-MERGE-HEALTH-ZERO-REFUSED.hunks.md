# DATA-MERGE-HEALTH-ZERO-REFUSED — HEAD re-break control patch

Patch: `docs/bugs/promotions/DATA-MERGE-HEALTH-ZERO-REFUSED.patch`, the fix hunks of `42ae17014`
(`data: accept Health: 0 in a merge and put the mob on the corpse clock`). It reverses on that tree
(`git apply -R --check` clean) and reverses the real fix only, not a test or a comment.

## What reverses (4 hunks in `command_summon.rs`, 2 in `command_data/entity.rs`)

1. `Image::lethal` field (with its doc comment) and `decode`'s `lethal: false` initialiser.
2. `prepare_edit`'s `zero_health` closure: the predicate that lets a zero `Health` past the summon
   whitelist (`authored_fields` refuses `v <= 0.0`, "Invalid Health") and records `lethal`.
3. `prepare_edit`'s `edited.lethal = lethal;` hand-off.
4. `command_data/entity.rs`: reading `image.lethal` before the image moves and, after `apply`, zeroing
   the row and starting the corpse clock (`body.alive = false; entities.begin_dying(slot)`).

## Red on the reversal

The reversed tree refuses `data merge entity <living mob> {Health:0f}` exactly as before the fix
(`Editing Health of minecraft:cow is not supported`? no: `Invalid Health` from `authored_fields`), so
`data_mutation` test `data_merge_health_zero_kills_a_living_mob_like_vanilla` fails at its first
assertion (`-1 != 1`) — an assertion, not a timeout. The rest of the command_data suite stays green,
so the control isolates this row.

## Failing first

Not recorded as a separate red run: the refusal is a one-line predicate in `authored_fields`
(`if v <= 0.0 || v > f32::MAX { return Err("Invalid Health") }`), and the new test's first assertion
is the same command the row reports as refused. A reversal run is the control.
