# TAGS-DUPLICATES: patch-mode fix identity

Original fix: the tag-set work landed across several commits (all non-test hunks are in
`docs/bugs/promotions/TAGS-DUPLICATES.patch`): `command_tag::normalized`/`validate_fields`/`normalize_fields`/
`normalize_opaque` (new module API), the ingress validation in `command_summon`, `command_data/entity` and
`command_storage::mutate`, the canonicalisation on native/WAL restore (`core/recovery.rs`,
`journal/replay.rs`) and the player/entity normalisation during checkpoint recovery and export
(`native_checkpoint.rs`). Before them vanilla `Tags:['d1','d1']` survived a summon and one `/tag … remove`
left the tag present.

The cumulative patch committed earlier at `docs/bugs/fix-patches/TAGS-DUPLICATES.patch` no longer reversed on
HEAD: its two `command_data/entity.rs` hunks carry the older `previous`/`edited` merge base, and a later commit
replaced that with `image`/`base` (the private-field merge base). Eleven hunks are carried unchanged; the two
`entity.rs` hunks are rewritten at their HEAD locations with the same pre-image (the unvalidated
`mutate(..., |_| Ok(()))` call and the missing post-merge `validate_fields(&new)`). The patch therefore
`fix_commit = "patch:b4495dec8d60ce44db8de70c616c9fe6d6cb7782610da69ec5a70a51b9b90c98"` in
`docs/bugs/promotions/TAGS-DUPLICATES.patch` and reverse-applies on HEAD (with line offsets). Re-audit:
`python3 -m tools.harness ledger promote TAGS-DUPLICATES --fix-patch docs/bugs/promotions/TAGS-DUPLICATES.patch --dry-run`.

## Carried

- `command_tag.rs` — the new `normalized`, `validate_fields`, `normalize_fields`, `normalize_opaque` helpers and
  the `tag … list` path's normalisation.
- `command_summon.rs` — the `Tags` validation in `experience_orb`, `retained_tags` (with the normalised
  re-serialisation) and `authored_fields`; `validate_tags` becomes `pub(crate)`.
- `command_summon_data.rs` — `prepare_tags` writes the normalised list.
- `command_storage.rs` — `mutate` takes and calls `validate_source` for `merge` and `modify`.
- `command_data/entity.rs` — the HEAD-equivalent hunks: `mutate` validates the authored patch and the merged
  compound is validated before it is installed.
- `core/recovery.rs`, `journal/replay.rs`, `native_checkpoint.rs` — entity and player tag lists are
  canonicalised on native/WAL restore and before offline conversion or live hydration, and a trailing-bytes or
  duplicate-key image refuses instead of being published.

## Not carried

- Test files — (b) kept so the control compiles and runs the row's refs; they are the refs.
- Nothing else is omitted: every non-test hunk of the fix is carried.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:command_summon::tests::scoreboard_tags_tests::duplicate_tags_summon_load_add_and_remove_have_set_semantics`
  — the row's own symptom: with the patch reversed the loaded list keeps both `d1` entries and one removal
  leaves the tag present.
- `cargo:mc-core:command_summon::tests::scoreboard_tags_tests::duplicate_tags_keys_are_refused_by_summon_and_data_before_mutation`
  — the duplicate-`Tags`-field refusal disappears with the reversed `validate_fields` calls.
- `cargo:mc-core:command_summon::tests::scoreboard_tags_disk_tests::duplicate_tags_keys_in_checkpoint_refuse_recovery`
  and `…_in_wal_refuse_recovery` — the restore-time refusal disappears with the reversed normalisation.
- `cargo:mc-core:core::entity_state_persistence_tests::duplicate_tags_in_legacy_wal_normalize_on_entity_restore`
  and `cargo:mc-world-conversion:export_tests::export_refuses_ambiguous_player_tags_from_real_wal_without_publication`
  — legacy normalisation and the export refusal.
- `harness:entities:native-v2-ledger-tag-duplicates` — a real client summons a NoAI cow with
  `Tags:['dupx','dupx']`, proves the selector matches, removes the tag once (the server answers `Updated tags on 1 entities`), and then
  an `execute unless entity @e[type=minecraft:cow,tag=dupx] run say DUP_GONE` branch prints its marker
  (`tools/harness/backends/native_ledger_tag_duplicates.py`). Passed receipt: see the promotion JSON.
