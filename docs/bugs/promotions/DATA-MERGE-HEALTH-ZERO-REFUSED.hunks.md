# DATA-MERGE-HEALTH-ZERO-REFUSED — HEAD re-break control patch

Patch: `docs/bugs/promotions/DATA-MERGE-HEALTH-ZERO-REFUSED.patch`, the fix hunks of `42ae17014`
with the `LiveVillager` refactor of `0cf289c5c` folded in, i.e. every hunk that makes a zero `Health`
land (and nothing else). It reverses on the current tree (`git apply -R --check` clean).

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

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/command_summon.rs` `@@ -772,10 +775,22 @@` (adapted; origin `42ae17014`, discovered origin):
  - HEAD `};` — e15e34ecb data: let `data merge entity` write a villager's runtime state, as vanilla does; in origin^: yes

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
