# WAL-REDB-REPAIR-ROLLS-BACK-BEHIND-PRUNED-CHECKPOINT: patch-mode fix identity

Original fix: three commits of branch `wt/StoreMarker`, merged into the integration line by `30f03346f` and in
main `7e421d34c`: `9896162d3` (durable marker with store id and tick, written after the commit and before the
prune; startup, hot backup and restore refuse a disagreeing pair), `ad4705400` (the marker is independent of
the manifest; startup refuses a store behind it) and `04027091a` (one pure `verify` for every offline store
path; census, probe, compaction, worldgen upgrade, backup and restore refuse and never rewrite the marker).
`13deed939` and `9742adeba` on the same branch are the manifest format version
(`CHECKPOINT-MANIFEST-HAS-NO-FORMAT-VERSION`) and are not part of this row.

`fix_commit = "patch:e749161b0a6f48036bd7da2f35c04d4a06168b519bbec2dd9edc513d19ca359a"` names the HEAD
equivalents of those commits' non-test **enforcement** hunks in
`docs/bugs/fix-patches/WAL-REDB-REPAIR-ROLLS-BACK-BEHIND-PRUNED-CHECKPOINT.patch` (a diff from the pre-fix call
sites to the candidate; `git apply -R` restores them). Re-audit:
`python3 -m tools.harness ledger promote WAL-REDB-REPAIR-ROLLS-BACK-BEHIND-PRUNED-CHECKPOINT --fix-patch docs/bugs/fix-patches/WAL-REDB-REPAIR-ROLLS-BACK-BEHIND-PRUNED-CHECKPOINT.patch --dry-run`.
The patch is the net of the three commits (they overlap in `journal.rs`, `hot_backup.rs`, `native.rs`), with the
API hunks below left out so the kept tests still compile. The candidate is HEAD `7e421d34c`; the marker files of
the three commits did not change between `04027091a` and `7e421d34c` (only `census_tests.rs`), so each hunk
applies verbatim.

Carried (reversing them turns the enforcement off at every call site):
| file | what the reversal removes |
|---|---|
| `mc-core/src/core.rs` (Core::open) | the first-start bootstrap publish and `store_marker::check` before replay |
| `mc-core/src/core/journal.rs` | `write_checkpoint_marker` and its two call sites (the writer loop, `write_checkpoint`); the marker-before-ACK/prune ordering, `committed_tick` stored only after a durable marker, the halt on a marker failure |
| `mc-core/src/hot_backup.rs` | the marker read, the store-id and tick refusals, the marker copy |
| `mc-core/src/native_checkpoint.rs` | `verify_marker` in `recover_offline` |
| `mc-edge/src/backup.rs` | `run_backup` pair verify; `run_restore` marker/store pair check and cleanup |
| `mc-world-conversion/src/{census,probe}.rs` | `verify_marker` before reading |
| `mc-world-conversion/src/{crash_census,custody_census}.rs`, `mc-world-tools/src/{pregenerate,world_maintenance}.rs` | `open_checked` back to `NativeStore::open` |
| `mc-world-tools/src/worldgen_upgrade.rs` | `check_marker_file` before the identity read |
| `mc-world-conversion/src/import.rs` | the converter's marker write |

Left out of the patch, each kept in the control tree:
| original hunk | reason |
|---|---|
| `mc-world/src/store_marker.rs` (new module: read/write/check/verify, `StoreMarker`, `MARKER_FILE`) | (b) the kept tests in mc-world, mc-core, mc-edge, mc-world-conversion and mc-world-tools call it |
| `mc-world/src/lib.rs` `pub mod store_marker` and the test module line | (b) |
| `mc-world/Cargo.toml`, `Cargo.lock` (uuid v4, sha2) | (b) `store_marker.rs` needs them |
| `mc-world/src/native.rs` (`store_uuid` field and header row, `NATIVE_DATABASE`, `NativeError::Marker`, `marker_fields`, `verify_marker`, `open_checked`, `check_marker_file`, `validate_header` returning the id) | (b) tests construct `NativeStore { store_uuid }` and call these; they enforce nothing until a caller calls them |
| `mc-world/src/native/checkpoint.rs` (`tick_from`, `marker_fields_from`, `checkpoint_tick`, `read_marker_fields`) | (b) |
| `mc-core/src/core.rs` `FAIL_CHECKPOINT_MARKER` and the `store_marker_tests` module line | (b) test hook and module registration |
| `mc-core/src/core/journal.rs` `native_failed` parameter of `run_native_work`; `mc-core/src/hot_backup.rs` `store_uuid` parameter of `copy_world` | (b) the kept tests pass them; the reversed bodies leave both unused (a warning, no behaviour) |
| `mc-world-conversion/src/test_support.rs` marker writes in `fixture_store` / `write_manifest` | (b) fixture setup the kept tests rely on |

The two parameters are in the a/ side and the b/ side alike; the patch adds no a/-side line outside the original
pre-image.

Proving refs, in the control (`control_tree: head`, the same tree with the patch reversed): 11 of 12 refs go
red, one stays green.
- `hot_backup_tests::copy_refuses_a_marker_ahead_of_the_committed_checkpoint` and
  `copy_refuses_a_marker_from_another_store`: `hot_backup_tests.rs:119` / `:140`, `unwrap_err()` on an `Ok`
  backup: without the check a store that rolled back behind its marker is copied as a valid backup.
- `census_keeps_a_store_ahead_of_its_marker_byte_identical_and_refuses_a_bad_pair` (`census_tests.rs:546`) and
  `probe_keeps_a_store_ahead_of_its_marker_byte_identical_and_refuses_a_foreign_pair` (`probe_tests.rs:379`):
  the offline report is `PASS` for a bad pair.
- `compact_refuses_a_store_that_disagrees_with_its_checkpoint_marker` (`world_maintenance_tests.rs:216`),
  `a_stale_marker_with_a_refused_upgrade_leaves_the_world_unchanged` (`worldgen_upgrade_tests.rs:254`) and the
  two mc-edge backup refs (`backup_tests.rs:547`, `:578`): the tools do not refuse.
- `core::store_marker_tests::*` go red at `store_marker_tests.rs:21` and `:296`, the expectation that a world
  with a checkpoint has a marker: the writer publishes none. That is the "nothing durable records the newer
  checkpoint" half of the symptom, but it fails before the rolled-back reopen, so the startup refusal itself
  (`Core::open`) is proved by the green restore run, not by a distinct red assertion; it is not relied on as
  the proving assertion.
- `mc-world store_marker_tests::store_behind_the_marker_is_refused_and_names_both_ticks` tests the pure kept
  module and stays green by design.

Not carried because it is not this row: the manifest-format-version commits `9742adeba` and `13deed939`.
