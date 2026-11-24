# CHUNK-REFUSAL-BLOCKS-CHECKPOINTS: patch-mode fix identity

Original fix: `1b84f73d` "fix(core): preserve custody when refusing corrupt chunks".
`fix_commit = "patch:7b7f5f553372e75f6678f98bed38ecdceabe2a738f92d54d617fb7cede83b721"`
names the HEAD equivalents of that fix's non-test hunks in
`docs/bugs/promotions/CHUNK-REFUSAL-BLOCKS-CHECKPOINTS.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code).
Re-audit:
`python3 -m tools.harness ledger promote CHUNK-REFUSAL-BLOCKS-CHECKPOINTS --fix-patch docs/bugs/promotions/CHUNK-REFUSAL-BLOCKS-CHECKPOINTS.patch --dry-run`

This supersedes `patch:dda9bcbe…` (which carried a synthetic `world.rs:705`
hunk moving `block_entity_dirty.insert` inside the residency `if`; the a/-side
now equals HEAD there, class (b)), and `patch:121feead…`, whose a/-side no longer compiled on the tip
(a later test reads `world.block_entity_dirty`), and the earlier
`patch:ae9b3c45…`: its hunks no longer
reversed on the candidate (a later refactor removed the `SaveGenerated` work
item and added the test-only `ContainerCaptureWork` field), so the patch was
regenerated against the tip. Every original non-test hunk is carried unless it
is another row's or owner's behaviour (a), API the kept tests call (b), or code
a later commit rewrote with no HEAD equivalent (c); each exclusion is named
below.

Carried (13 of the commit's 23 non-test hunks):

| original | HEAD |
|---|---|
| capture.rs -4,6 `quarantine` | capture.rs:31-80 |
| capture.rs -157,11 refused/declined at capture start | capture.rs:231-249 |
| capture.rs -169,7 inhabited skips failed | capture.rs:255-257 |
| capture.rs -200,9 homeless materialize no longer sets load_error | capture.rs:287-289 |
| capture.rs -211,11 dirty_absent failure + second quarantine | capture.rs:297-311 |
| capture.rs -227,6 debt skips failed | capture.rs:323-325 |
| capture.rs -276,6 metadata of quarantined chunk not written | capture.rs:393-394 |
| capture.rs -341,6 `block_entity_dirty` cleared on flush | capture.rs:461 |
| world.rs -724,8 `defer_checkpoint_chunk` counts instead of load_error | world.rs:758-767 |
| residency.rs -282,7 doc | residency.rs:285 |
| residency.rs -298,9 reject keeps dirty_sections, clears dirty_absent | residency.rs:301-306 |
| journal.rs -106,16 declined/skipped logging | journal.rs:204-234 |
| mc-world chunk_nbt.rs -369,6 importer: the block owns its record | chunk_nbt.rs:372-395 |

Not carried (10 of 23), all reason (b) — one list:

1. `world.rs` -1375,6 (`RefusedChunk` + Display), -1382,10
   (`ChunkCapture.refused` field + `refused()`) and `residency.rs` -406,6
   (`refused: Vec::new()` in the generated capture): the kept chunk_loader
   tests call `capture.refused()[0].unsaved/.scheduled`. Adaptation that
   follows: the reversed capture start initialises the kept field as
   `refused: Vec::new()` (capture.rs:228-249 a/ side; the only a/ line that is
   not in the original pre-image), so `refused()` stays empty in the control.
2. `Box<ChunkCapture>` in `core.rs` (`JournalWork::RequireGenerated.capture`),
   `core/journal.rs` (`NativeWork::RequireGenerated.chunks`) and the two boxing
   sites in `core/tick_loop.rs`: the kept tests construct the boxed form —
   `core_journal_backpressure_tests.rs:413` and `:839` send
   `JournalWork::RequireGenerated { capture: Box::new(invalid), .. }`.
3. `world.rs` `block_entity_dirty` (field, initializer, insert in
   `mark_block_entity_dirty`): the kept `world_capture_tests.rs:354` reads
   `world.block_entity_dirty`.

The compile evidence: reversing any of them fails the control build
(E0308 at the two backpressure lines, E0609 at the world_capture line;
observed during this revision) and HEAD keeps both shapes, so these are
kept-API exclusions, not missing HEAD equivalents.

The a/-side compiles on the tip: plain `cargo check -p mc-core --tests` on the
reversed tree at `HEAD 5fd0d7dd04` (a-side blobs from `patch:7b7f5f55`),
command `git apply -R docs/bugs/promotions/CHUNK-REFUSAL-BLOCKS-CHECKPOINTS.patch
&& cargo check -p mc-core --tests`; log (with its reconciliation header) at
`receipts/D123/D123Persist/review/CHUNK-aside-cargo-check.log`. No harness
profile wraps a bare check.

## a/-side adaptation

`capture.rs` start (HEAD line 231 hunk of the old identity) keeps the fixed
`declined`/`refused` block on the b/ side and the pre-fix `load_error: None`,
`refused: Vec::new()` on the a/ side; HEAD's test-only `container_work:
ContainerCaptureWork::default()` field is untouched on both sides, which is why
the old hunk no longer matched its context.

## Proving ref

`chunk_loader::tests::refusing_a_resident_chunk_keeps_custody_of_its_unsaved_edits`
goes red at chunk_loader_tests.rs:886 `assert!(capture.load_error().is_none())`:
a chunk refused as freshly loaded, with nothing unsaved, declines the
checkpoint, which is this row's symptom. Not evidence:
`refused_chunk_never_blocks_other_chunks_and_stays_refused` red at :804
(`refused().len() == 1`, left 0) and the unregistered
`refusing_a_resident_chunk_keeps_custody_of_folded_block_edits` red at :954 only
assert the kept `refused()` API, which the adaptation leaves empty; the bug
assertion of the first, :802 `load_error().is_none()`, passes in the control.
`promote.json` records both registered cargo refs as plain `FAIL`; only :886
counts.

## Original non-test hunks

```
## 1b84f73d non-test hunks (kept test files: crates/mc-core/src/chunk_loader_tests.rs, crates/mc-core/src/core_checkpoint_custody_tests.rs, crates/mc-world/src/anvil/chunk_nbt_tests.rs)
- crates/mc-core/src/core.rs: -212,7 +212,7 enum JournalWork {
- crates/mc-core/src/core/journal.rs: -106,16 +106,46 pub(super) fn start_checkpoint_with_reply(; -532,7 +562,7 pub(super) enum NativeWork {
- crates/mc-core/src/core/tick_loop.rs: -288,7 +288,9 pub(super) fn tick_loop(mut state: TickState, mut ; -587,7 +589,10 pub(super) fn send_shutdown(
- crates/mc-core/src/world.rs: -76,6 +76,11 pub struct ChunkWorld {; -496,6 +501,7 impl ChunkWorld {; -675,6 +681,7 impl ChunkWorld {; -724,8 +731,16 impl ChunkWorld {; -1375,6 +1390,27 struct DeferredChunkCapture {; -1382,10 +1418,18 pub struct ChunkCapture {
- crates/mc-core/src/world/capture.rs: -4,6 +4,56; -157,11 +207,20 impl ChunkWorld {; -169,7 +228,9 impl ChunkWorld {; -200,9 +261,7 impl ChunkWorld {; -211,11 +270,21 impl ChunkWorld {; -227,6 +296,9 impl ChunkWorld {; -276,6 +348,11 impl ChunkWorld {; -341,6 +418,7 impl ChunkWorld {
- crates/mc-core/src/world/residency.rs: -282,7 +282,7 impl ChunkWorld {; -298,9 +298,12 impl ChunkWorld {; -406,6 +409,7 impl ChunkWorld {
- crates/mc-world/src/anvil/chunk_nbt.rs: -369,6 +369,30 fn chunk_from_nbt_with_items_impl(
```
