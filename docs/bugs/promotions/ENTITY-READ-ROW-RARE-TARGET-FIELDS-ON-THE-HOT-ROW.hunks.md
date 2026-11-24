# ENTITY-READ-ROW-RARE-TARGET-FIELDS-ON-THE-HOT-ROW — fix-hunk map

The production-only patch `ENTITY-READ-ROW-RARE-TARGET-FIELDS-ON-THE-HOT-ROW.patch` is the fix's
non-test hunks at HEAD, SHA-256 of its non-test hunks
`patch:f1ca3154281b4dffd52d5cd3f47a9344ed7f0d51df7c7d8b5ee06baf1929594c`. It reverse-applies on the
candidate (`git apply -R --check` on `5f3415769`, clean tree).

## The fix commit and the hunks the patch carries

The fix is `5f3415769` (`perf(core): one rare motion target per ReadRow; indexed battle faction
tables`), contained in this tree. It touched, for this row:

| file | hunk | in the patch? |
|---|---|---|
| `crates/mc-core/src/tick/entity_phase.rs` | removes `plugin_target`/`gold_target` (`Option<[f64; 3]>`, 32 B each) from `ReadRow`, adds `RareMotion` and the `rare_motion` field, rewrites the `think`-filter read | **removal only** (the two fields are re-added by the reversed hunk) |
| `crates/mc-core/src/tick/entity_phase/prepare.rs` | `read_row` builds `rare_motion` from the two rare targets instead of copying them | **the two `plugin_target: None` / `gold_target: None` lines** are added by the reversed hunk |
| `crates/mc-core/src/tick/entity_phase/step.rs` | `row.plugin_target`/`row.gold_target` reads become `RareMotion::Plugin`/`::Gold` matches | no — kept (see below) |
| `crates/mc-core/src/entity_phase_tests.rs` | exact size ratchet `== 224`, `rare_motion` match | no — test file (kept whole by the splitter) |
| `crates/mc-core/src/tick/system_graph_tests.rs` | drops the duplicate `<= 256` bound, fixes the row name | no — test file |

## The a/-side against the real pre-image

The reversed tree is compared against the file at `5f3415769^`. Every difference:

1. **`RareMotion` (enum, field, `read_row` wiring, the `think` filter) is kept.** The kept sibling test
   `entity_phase_tests::parallel_row_prep_matches_the_serial_prep_including_rare_fixups` reads
   `row.rare_motion` (it is the fix's retarget of the old `plugin_target.is_some()` assertion), so a
   control without the enum or the field stops compiling — README rule (b). The a/-side therefore sets
   `plugin_target: None, gold_target: None` beside the kept `rare_motion` initializer: same behaviour
   (`step.rs` and the filter read only `rare_motion`; the two restored fields are `#[allow(dead_code)]`
   and nothing reads them), and the exact 64 B the row describes come back on the struct.
2. **Field order.** The a/-side puts the two restored fields before `rare_motion`, where the pre-image
   had them (`standing`, `plugin_target`, `gold_target`, `leash_follow`); the ratchet is on the total
   size, so order does not matter, but this keeps the diff readable.
3. The `#[allow(dead_code)]` attributes on the two restored fields are new text: without them the control
   would warn on dead fields. They do not change layout or behaviour.

No other text differs: `read_row`'s remaining initializer, `step.rs`, the enum and the filter are
HEAD's.

## Why this control is honest

- The registered ref is `cargo:mc-core:tick::entity_phase::entity_phase_tests::worker_snapshot_keeps_rare_state_in_entity_store`,
  whose only assertion is `assert_eq!(std::mem::size_of::<ReadRow>(), 224, …)`
  (`crates/mc-core/src/entity_phase_tests.rs:463-465`). With the two 64 B fields restored the struct is
  288 B, so the assertion fails at the row's own size claim — not at a refusal, a precondition or a
  compile error (the control keeps every type, field and initializer the kept tests use).
- The reversal is the fix's storage hunk: exactly the two fields the row names. `step.rs` readers are
  not reversed, because the row's cost is fill/cache footprint, not the readers, and reversing them is
  a behaviour change no registered ref measures.
- An exact number (`== 224`), not a `<=` bound: the earlier `<= 256` accepted the 64 B the row is about.
  The 224 B is the fixed layout (`RareMotion` is 32 B, the two fields were 64 B).
- The claim is storage only: 64 B per admitted row less in the prepare output. No fill-rate or
  wall-clock claim is made.

## Observed control and restore

`ledger promote ENTITY-READ-ROW-RARE-TARGET-FIELDS-ON-THE-HOT-ROW`, receipt
`.analysis/validation/20261124T011809-ledger-promote-vaqetzwm` on `5f3415769` (`tree_dirty: false`,
`control_tree: head`, `reversed_files` = `tick/entity_phase.rs`, `tick/entity_phase/prepare.rs`):

- control (`ledger-1-run.log:268-270`) **FAIL** —
  `assertion `left == right` failed: hot combat/breeding scalar snapshot copies 288 bytes; the exact
  bound is 224`, `left: 288`, `right: 224` — the row's own size assertion at
  `entity_phase_tests.rs:463`, not a refusal, precondition or compile break;
- restore green (`ledger-3-run.log`, exit 0).

## What stays out of the row

`ReadRow.enderman_target` (`EntityId`) is untouched by the fix and is not part of the 64 B. The rare
side vector `PrepChunk.rare` (`Vec<usize>` of row indices) is unchanged: the fix keeps `read_row`
reading rare fields from `ColdRow` by slot at prepare time, so no second accessor or cache was added.
