# TAGS-DUPLICATES: patch-mode fix identity

## Origin commits

Three commits close this row's path; the patch carries the non-test hunks of all three.

| commit | what it closes |
|---|---|
| `c224df8de` (`fix(core): normalize scoreboard tags and explain summon refusals`) | `command_tag::normalized` and `normalize_opaque`; the summon/`prepare_tags`/`retained_tags` normalisation; `tag <targets> list`; the first recovery/replay call sites |
| `0c51b3a44` (`fix: fence stale world frames and reject ambiguous tag records`) | `command_tag::validate_fields`; the `mutate` source validator; the `data merge/modify entity` validation (both the authored patch and the merged compound); `entity.rs`'s `Tags` branch moves to `command_summon::validate_tags`; `validate_tags` becomes `pub(crate)` |
| `043bf0302` (`fix: close retained WAL and profile selector parsing gaps`) | the trailing-bytes/duplicate-key refinement in the retained-WAL and checkpoint call sites (`core/recovery.rs`, `native_checkpoint.rs`) |

Before them vanilla `Tags:['d1','d1']` survived a summon and one `/tag … remove` left the tag present.

The cumulative patch committed earlier at `docs/bugs/fix-patches/TAGS-DUPLICATES.patch` did not reverse on HEAD:
its `command_data/entity.rs` hunks carry the older `previous`/`edited` merge base and its `command_tag.rs` hunk
added the whole helper block. This patch rewrites the `entity.rs` hunks at HEAD's `image`/`base` lines and keeps
the helper block in place. It is
`fix_commit = "patch:df4639c038eb3a69d8b66e8bf9a6db3e7fbf0cb48023fc9eeb37eb2e1c0cee47"` and reverse-applies on
HEAD. Its `index` blob ids and `@@` line numbers are inherited from the old cumulative patch and are shifted
relative to this tree (for example `native_checkpoint.rs` `-853` vs HEAD `:1088`, `core/recovery.rs` `-590` vs
`:613`); git applies every hunk on those shifts, and the reversal was checked with
`git apply -R --check` on the promoted tree. Re-audit:
`python3 -m tools.harness ledger promote TAGS-DUPLICATES --fix-patch docs/bugs/promotions/TAGS-DUPLICATES.patch --dry-run`.

## Carried

- `command_summon.rs` — `validate_tags` called from `experience_orb`, `retained_tags` (with the normalised
  re-serialisation of the retained compound) and `authored_fields`, and `validate_tags` made `pub(crate)`.
- `command_summon_data.rs` — `prepare_tags` writes the list through `command_tag::normalized`.
- `command_storage.rs` — `mutate` takes and calls `validate_source` for `merge` and `modify`.
- `command_data/entity.rs` — three hunks at HEAD: `mutate` validates the authored patch
  (`entity.rs:81`), the merged compound is validated before it is installed (`entity.rs:91`), and the `Tags`
  branch validates through `command_summon::validate_tags` instead of its own inline block.
- `command_tag.rs` — the `tag <targets> list` path returns the normalised element list.
- `core/recovery.rs`, `journal/replay.rs`, `native_checkpoint.rs` — entity and player tag lists are
  canonicalised through `command_tag::normalize_opaque` on native/WAL restore, before offline conversion or
  live hydration.

### a/-side adaptations (class b, named)

- `command_storage.rs` patch line 56: the reverted `mutate` signature has `_validate_source` (five parameters).
  The pre-image `mutate` had **no** validator at all (`command_storage.rs` took five parameters before
  `c224df8de`/`0c51b3a44`); the underscore names the unused parameter so the reversed tree compiles for its kept
  callers. No behaviour.
- `command_data/entity.rs` patch line 7: the reverted call is
  `mutate(state, context, action, &mut edited, &mut input, |_| Ok(()))` — the exact pre-image call
  (`entity.rs:71` before `0c51b3a44`) — because the fix is the closure body, not the closure's existence.

## Not carried

- `command_tag.rs`'s helper definitions (`normalized`, `validate_fields`, `normalize_fields`,
  `normalize_opaque`) — (b) kept API: their only non-carried caller is
  `crates/mc-core/src/settlements/transit.rs:1043` (`normalize_opaque` for settlement transit members — (a)
  transit behaviour, not this row). Both `entity.rs` call sites (`:81`, `:91`) are inside carried hunks, so
  leaving the pure helpers in the reversed tree changes no tag behaviour this row observes.
- Test files — (b) kept so the control compiles and runs the row's refs; they are the refs.

## Proving refs (file:line of the failing assertion)

Evidence from the promotion receipt `20261124T140410-ledger-promote-3ffesjtc` (all ten refs green on the
restore).

Red, and the row's own symptom:
- `cargo:mc-core:command_summon::tests::scoreboard_tags_tests::duplicate_tags_summon_load_add_and_remove_have_set_semantics`
  — `crates/mc-core/src/command_summon_scoreboard_tags_tests.rs:31`: the loaded list keeps both `d1` entries and
  one removal leaves the tag present.
- `cargo:mc-core:command_summon::tests::scoreboard_tags_tests::duplicate_tags_keys_are_refused_by_summon_and_data_before_mutation`
  — `…/command_summon_scoreboard_tags_tests.rs:174`: the duplicate-`Tags`-field refusal disappears.
- `cargo:mc-core:command_summon::tests::scoreboard_tags_disk_tests::duplicate_tags_checkpoint_and_wal_recovery_is_idempotent`
  — `…/command_summon_scoreboard_tags_disk_tests.rs:116`: the normalised image is no longer stable.
- `cargo:mc-core:command_summon::tests::scoreboard_tags_disk_tests::duplicate_player_tags_in_wal_normalize_before_offline_export`
  — `…/command_summon_scoreboard_tags_disk_tests.rs:261`: duplicate player elements survive into the exported
  image.
- `cargo:mc-core:core::entity_state_persistence_tests::duplicate_tags_in_legacy_wal_normalize_on_entity_restore`
  — `crates/mc-core/src/entity_state_persistence_tests.rs:1660`: a legacy WAL entity restores with both entries.

Green in the control, **not this patch's** evidence:
- `cargo:mc-core:command_summon::tests::scoreboard_tags_disk_tests::duplicate_tags_keys_in_checkpoint_refuse_recovery`
  — green with the patch reversed: that refusal comes from the checkpoint decoder
  (`native checkpoint/NBT/duplicate_field`, `command_summon_scoreboard_tags_disk_tests.rs:179-180`), which is
  outside this patch. "The refusal disappears with the reversed normalisation" was wrong.

Not evidence (control could not run the ref):
- `…::duplicate_tags_keys_in_wal_refuse_recovery` (`disk_tests.rs:197`),
  `…::opaque_wal_trailing_bytes_refuse_offline_and_live_recovery` (`disk_tests.rs:218`),
  `…::duplicate_player_tags_keys_in_wal_refuse_offline_export_and_live_recovery` (`disk_tests.rs:296`) and
  `cargo:mc-world-conversion:export_tests::export_refuses_ambiguous_player_tags_from_real_wal_without_publication`
  (`export_tests.rs:684`) all report `fixture-failure` in the reversed control (the fixture world/image cannot
  build there), and `harness:entities:native-v2-ledger-tag-duplicates` is `unrunnable` in the control for lack
  of a reverted-tree pin (POL-1 practice). Its passed candidate receipt is the graphical half.

## Real-client ref

`harness:entities:native-v2-ledger-tag-duplicates` — a real client summons a NoAI cow with
`Tags:['dupx','dupx']`, proves the selector matches (`execute if … run say DUP_PRESENT`), removes the tag once
(the server answers `Updated tags on 1 entities`), and then
`execute unless entity @e[type=minecraft:cow,tag=dupx] run say DUP_GONE` prints its marker, which a tree that
kept the second entry cannot produce (`tools/harness/backends/native_ledger_tag_duplicates.py`). Passed:
`20261124T135720-native-v2-ledger-tag-duplicates-_3enoigi` on `3dd8ddf19d`.
