# TAGS-DUPLICATES: patch-mode fix identity

Original fix: the tag-set work landed across several commits. All non-test hunks are in
`docs/bugs/promotions/TAGS-DUPLICATES.patch`: the ingress validation in `command_summon`, `command_data/entity`
and `command_storage::mutate`, the canonicalisation on native/WAL restore (`core/recovery.rs`,
`journal/replay.rs`) and the player/entity normalisation during checkpoint recovery (`native_checkpoint.rs`).
Before them vanilla `Tags:['d1','d1']` survived a summon and one `/tag … remove` left the tag present.

The cumulative patch committed earlier at `docs/bugs/fix-patches/TAGS-DUPLICATES.patch` did not reverse on HEAD:
its `command_data/entity.rs` hunks carry the older `previous`/`edited` merge base and it also carried the whole
`command_tag.rs` helper block. The patch here rewrites the `entity.rs` hunks at their HEAD locations (same
pre-image: the unvalidated `mutate(..., |_| Ok(()))` call, the missing post-merge validation and the old inline
`Tags` block) and keeps the helper block in place. It is
`fix_commit = "patch:df4639c038eb3a69d8b66e8bf9a6db3e7fbf0cb48023fc9eeb37eb2e1c0cee47"` and reverse-applies on
HEAD (with line offsets). Re-audit:
`python3 -m tools.harness ledger promote TAGS-DUPLICATES --fix-patch docs/bugs/promotions/TAGS-DUPLICATES.patch --dry-run`.

## Carried

- `command_summon.rs` — `validate_tags` called from `experience_orb`, `retained_tags` (with the normalised
  re-serialisation of the retained compound) and `authored_fields`, and `validate_tags` made `pub(crate)`.
- `command_summon_data.rs` — `prepare_tags` writes the list through `command_tag::normalized`.
- `command_storage.rs` — `mutate` takes and calls `validate_source` for `merge` and `modify`.
- `command_data/entity.rs` — three hunks at HEAD: `mutate` validates the authored patch, the merged compound is
  validated before it is installed, and the `Tags` branch validates through `command_summon::validate_tags`
  instead of its own inline block.
- `command_tag.rs` — the `tag <targets> list` path returns the normalised element list.
- `core/recovery.rs`, `journal/replay.rs`, `native_checkpoint.rs` — entity and player tag lists are
  canonicalised through `command_tag::normalize_opaque` on native/WAL restore, before offline conversion or
  live hydration; a trailing-bytes or duplicate-key image refuses instead of being published.

## Not carried

- `command_tag.rs`'s helper definitions (`normalized`, `validate_fields`, `normalize_fields`,
  `normalize_opaque`) — (b) kept API: `crates/mc-core/src/settlements/transit.rs:1043` calls `normalize_opaque`
  for settlement transit members (not this row), and the kept `command_data/entity.rs` path calls
  `validate_fields`. Leaving the pure helpers in the reversed tree changes no behaviour; every call site of
  theirs that this row observes is carried above.
- Test files — (b) kept so the control compiles and runs the row's refs; they are the refs.

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
  `Tags:['dupx','dupx']`, proves the selector matches, removes the tag once (the server answers
  `Updated tags on 1 entities`), and then an `execute unless entity @e[type=minecraft:cow,tag=dupx] run say
  DUP_GONE` branch prints its marker (`tools/harness/backends/native_ledger_tag_duplicates.py`). Passed
  receipt: see the promotion JSON.
