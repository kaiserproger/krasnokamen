# CHUNK-REFUSAL-BLOCKS-CHECKPOINTS: patch-mode fix identity

Original fix: 1b84f73d. `fix_commit = "patch:ae9b3c453c1f32ffffa901dbfbc56d7670f8ae05051fbe61b94126972a0deb90"` names the HEAD
equivalents of that fix's non-test hunks in `docs/bugs/promotions/CHUNK-REFUSAL-BLOCKS-CHECKPOINTS.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote CHUNK-REFUSAL-BLOCKS-CHECKPOINTS --fix-patch docs/bugs/promotions/CHUNK-REFUSAL-BLOCKS-CHECKPOINTS.patch --dry-run`
(plus `--client-receipt` for a real-client row). Rule (docs/bugs/README.md, step 1): every original
non-test hunk is carried unless it is another row's or owner's behaviour, API the kept tests call,
or code a later commit rewrote; each exclusion is named below.

Carried (20 of the commit's 23 non-test hunks):
| original | HEAD |
|---|---|
| capture.rs -4,6 `quarantine` | capture.rs:31-80 |
| capture.rs -157,11 refused/declined at capture start | capture.rs:234-249 |
| capture.rs -169,7 inhabited skips failed | capture.rs:255-257 |
| capture.rs -200,9 homeless materialize no longer sets load_error | capture.rs:287-289 |
| capture.rs -211,11 dirty_absent failure + second quarantine | capture.rs:297-311 |
| capture.rs -227,6 debt skips failed | capture.rs:323-325 |
| capture.rs -276,6 metadata of quarantined chunk not written | capture.rs:375-379 |
| capture.rs -341,6 `block_entity_dirty` cleared on flush | capture.rs:445 |
| world.rs -76,6 `block_entity_dirty` field | world.rs:79-83 |
| world.rs -496,6 its init | world.rs:495 |
| world.rs -675,6 its insert in `mark_block_entity_dirty` | world.rs:675 |
| world.rs -724,8 `defer_checkpoint_chunk` counts instead of load_error | world.rs:725-736 |
| residency.rs -282,7 doc | residency.rs:285 |
| residency.rs -298,9 reject keeps dirty_sections, clears dirty_absent | residency.rs:301-306 |
| journal.rs -106,16 declined/skipped logging | journal.rs:155-195 |
| journal.rs -532,7, core.rs -212,7, tick_loop.rs -288,7/-587,7 `Box<ChunkCapture>` | journal.rs:611, core.rs:215, tick_loop.rs:303-305, 604-607 |
| mc-world chunk_nbt.rs -369,6 importer: the block owns its record | chunk_nbt.rs:372-395 |

Not carried (3 of 23): world.rs -1375,6 (`RefusedChunk` + Display), world.rs -1382,10 (`ChunkCapture.refused` field + `refused()`), residency.rs -406,6 (`refused: Vec::new()` in the generated capture): (b) the kept chunk_loader tests call `capture.refused()[0].unsaved/.scheduled`; reversing them is why the `fix_commit` tree does not compile. Adaptation that follows: the reversed capture start initialises the kept field as `refused: Vec::new()` (capture.rs:242-249 a/ side; the only a/ line that is not in the original pre-image), so `refused()` stays empty in the control.

Proving ref: `chunk_loader::tests::refusing_a_resident_chunk_keeps_custody_of_its_unsaved_edits` goes red at chunk_loader_tests.rs:885 `assert!(capture.load_error().is_none())`: a chunk refused as freshly loaded, with nothing unsaved, declines the checkpoint, which is this row's symptom. Not evidence: `refused_chunk_never_blocks_other_chunks_and_stays_refused` red at :803 (`refused().len() == 1`, left 0) and the unregistered `refusing_a_resident_chunk_keeps_custody_of_folded_block_edits` red at :954 only assert the kept `refused()` API, which the adaptation leaves empty; the bug assertion of the first, :802 `load_error().is_none()`, passes in the control. `promote.json` records both registered cargo refs as plain `FAIL`; only :885 counts.

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
