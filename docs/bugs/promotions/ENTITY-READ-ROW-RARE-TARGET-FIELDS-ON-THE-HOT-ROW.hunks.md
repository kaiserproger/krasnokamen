# ENTITY-READ-ROW-RARE-TARGET-FIELDS-ON-THE-HOT-ROW — fix-hunk map

The production-only patch `ENTITY-READ-ROW-RARE-TARGET-FIELDS-ON-THE-HOT-ROW.patch` is the fix's
non-test hunks, SHA-256 of its non-test hunks
`patch:5aaaeaa28afe08bf33513cff40287b6721319fc7e345cca24264103cf8011f8b`. It reverse-applies on the
candidate (`git apply -R --check` on `071a68f8e4`, clean tree).

## The fix commit and the hunks the patch carries

The fix is `5f3415769` (`perf(core): one rare motion target per ReadRow; indexed battle faction
tables`), contained in this tree. It touched, for this row:

| file | hunk | in the patch? |
|---|---|---|
| `crates/mc-core/src/tick/entity_phase.rs` | removes `plugin_target`/`gold_target` (`Option<[f64; 3]>`, 32 B each) from `ReadRow` (pre-fix order `standing`, `plugin_target`, `leash_follow`, `gold_target`), adds `rare_motion`, rewrites the `think`-filter read `row.plugin_target.is_none()` | **yes** — the pre-fix fields (in their real order) and the real filter read come back |
| `crates/mc-core/src/tick/entity_phase/prepare.rs` | `read_row` initialises `plugin_target: rare.plugin_target, gold_target: rare.gold_target` and builds `rare_motion` from the same two targets | **yes** — the two real initialisers come back |
| `crates/mc-core/src/tick/entity_phase/step.rs` | the two readers `let Some(target) = row.plugin_target` / `row.gold_target` become `RareMotion::Plugin`/`::Gold` matches | **yes** — the real readers come back |
| `crates/mc-core/src/entity_phase_tests.rs` | exact size ratchet `== 224`, `rare_motion` match | no — test file (kept whole by the splitter) |
| `crates/mc-core/src/tick/system_graph_tests.rs` | drops the duplicate `<= 256` bound, fixes the row name | no — test file |

## The a/-side against the real pre-image

The reversed tree is compared against the files at `5f3415769^` (the later COLD-ROW commit is carried
unchanged by both sides and is not part of this patch). Every difference:

1. **`RareMotion` (enum, the `rare_motion` field and its `read_row` initialiser) is kept.** The kept
   sibling test `entity_phase_tests::parallel_row_prep_matches_the_serial_prep_including_rare_fixups`
   reads `row.rare_motion` (the fix's retarget of the old `plugin_target.is_some()` assertion), so a
   control without the enum or the field stops compiling — README rule (b). This is the ONLY adaptation:
   the enum, its field and the initialiser.
2. **Nothing else differs.** The pre-fix fields are restored with their real order (`standing`,
   `plugin_target`, `leash_follow`, `gold_target`), the real initialisers (`plugin_target:
   rare.plugin_target`, `gold_target: rare.gold_target`) and the real readers in `step.rs` and the
   `think` filter (`row.plugin_target.is_none()`). There is no invented comment, no
   `#[allow(dead_code)]`, no `None` initialiser: with real values both sides behave identically, so the
   control exercises the row's actual subject — the two 32 B fields on the hot struct.

## Why this control is honest

- The registered ref is
  `cargo:mc-core:tick::entity_phase::entity_phase_tests::worker_snapshot_keeps_rare_state_in_entity_store`,
  whose only assertion is `assert_eq!(std::mem::size_of::<ReadRow>(), 224, …)`
  (`crates/mc-core/src/entity_phase_tests.rs`). The control behaves like the fix (the restored readers
  consume the restored fields), and fails at the row's own size claim: the two fields return 64 B plus
  the kept `rare_motion` field, so the struct is 288 B — not a refusal, precondition or compile break.
- The pre-fix struct was 256 B — the fix removes two 32 B fields and adds one 32 B field:
  256 − 64 + 32 = 224, a 32 B saving per admitted row.
- The reversal covers every reader the row names: the filter, `step.rs:151`, `step.rs:254` and the two
  initialisers, so the control is the real pre-fix code, not a storage-only stub.
- An exact number (`== 224`), not a `<=` bound: the earlier `<= 256` accepted the 32 B the row is about.
- The claim is storage only: 32 B per admitted row less in the prepare output. No fill-rate or
  wall-clock claim is made.

## Observed control and restore

Earlier control (dishonest a/-side; superseded): `.analysis/validation/20261124T011809-ledger-promote-vaqetzwm`.
This regenerated patch's control is recorded below once re-promoted.

## What stays out of the row

`ReadRow.enderman_target` (`EntityId`) is untouched by the fix and is not part of the 32 B. The rare
side vector `PrepChunk.rare` (`Vec<usize>` of row indices) is unchanged: the fix keeps `read_row`
reading rare fields from `ColdRow` by slot at prepare time, so no second accessor or cache was added.
