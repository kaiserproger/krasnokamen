# ANTI-XRAY-BORDER-REVEAL-LOST-ACROSS-SKIPPED-PUBLISH — fix-hunk map

The production patch `ANTI-XRAY-BORDER-REVEAL-LOST-ACROSS-SKIPPED-PUBLISH.patch` is the fix's
non-test hunks, `patch:5aa0532138099cdaa0d3ee6aab7cdce18bae540de4898edecced65b03120f1d6`. It
reverse-applies on the candidate (`git apply -R --check`, clean tree at `4aad4c22c`).

## The fix, in two commits

| commit | what it closes |
|---|---|
| `523b97aa5` fix(edge): repair anti-xray border reveals across skips | `ViewerState::skip_publication` keeps the resident chunks of the last publish before the first skip; the next plan re-sends a held chunk whose side neighbour moved since, under anti-xray only. `ChunkDeltaLog::previous` reads that base. |
| `42b543746` fix(edge): count side neighbour loads and unloads | `side_moved` compares `Option<revision>` per side, so a neighbour that loaded or unloaded during the skip also re-sends the held chunk (anti-xray treats an unloaded side as open). |

No later commit touched `crates/mc-edge/src/plan/{chunk_delta,viewer,viewer_plan}.rs`.

## Hunks the patch carries

| file | hunk | in the patch? |
|---|---|---|
| `crates/mc-edge/src/plan/chunk_delta.rs` | `pub(super) fn previous(&self, dimension) -> Option<&Arc<Vec<SnapshotChunk>>>` | **yes** |
| `crates/mc-edge/src/plan/viewer.rs` | `skip_base: Option<Arc<Vec<SnapshotChunk>>>` field + `None` in `new`, and `skip_publication` capturing the base once per skip run | **yes** |
| `crates/mc-edge/src/plan/viewer_plan.rs` | `let skip_base = viewer.skip_base.take();`, the `side_reveals` gate (`skipped && anti_xray enabled`) inside the unchanged-revision arm, and `side_moved` (the `Option<revision>` form of `42b543746`) | **yes** |
| `crates/mc-edge/src/server/pump.rs` | the two `skip_publications` signatures now pass `chunk_cache` and call `viewer.skip_publication(chunk_cache)` | no — the call site is unchanged relative to the patch: at HEAD the pump already calls `viewer.skip_publication(chunk_cache)`, and the patch's a/-side keeps that method (below). Reversing the pump would also revert `bfdc23f1c`'s stale/queue-budget behaviour, which is `EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS` (reason (a)). |
| `crates/mc-edge/src/anti_xray_tests.rs` | the three registered tests plus `border_ore`/`frames_across_skip` helpers | no — test file (kept whole by the splitter) |
| `crates/mc-edge/src/plan/chunk_delta.rs` / docs (`architecture/runtime.md`, ADR 0013), `admin_tests.rs`, `core_halt_tests.rs`, `explosion_queue_tests.rs`, `transfer_tests.rs`, `auth_edge_fence_tests.rs` | the pump-gate/join-linkage/doc changes of the same commit | no — pump-gate/join-linkage test wiring belongs to `EDGE-*` rows (a); docs are skipped by the splitter (c: doc prose) |

## The a/-side against the real pre-image

The reversed tree is the pre-image text of `523b97aa5^` for all three files, with one named
constructor line:

1. **`ViewerState::skip_publication` body.** The pre-image had no method: the pump set
   `viewer.skipped = true` inline (`523b97aa5^:crates/mc-edge/src/server/pump.rs:88`). The three
   registered tests call `v.skip_publication(&cache)`, so the control must keep the method
   (README rule (b), the kept-API constructor class). The a/-side is exactly the pre-image behaviour,
   one line:

   ```rust
   pub(crate) fn skip_publication(&mut self, _chunk_cache: &ChunkPacketCache) {
       self.skipped = true;
   }
   ```

   `_chunk_cache` is unused on that side; no field, no capture, no branch the pre-image never ran.
   The pre-image imports nothing extra: `viewer.rs` uses `use super::*;`, so `ChunkPacketCache`
   resolves on both sides. Its two-line a/- doc comment ("The pump skipped an output: the pre-image
   skip flag; every chunk whose own revision moved is sent again by the next plan.") is an added
   line, named here (review B2, 2026-11-24).
2. Nothing else: `skip_base`, `previous()`, `side_reveals` and `side_moved` are absent from the
   a/-side, and the doc comment of `skipped` is the pre-image sentence.

## Proving refs

| ref | assertion that is the row's symptom |
|---|---|
| `cargo:mc-edge:anti_xray::anti_xray_tests::a_border_reveal_inside_a_skipped_publish_reaches_the_client` | the ore chunk is sent again with the real ore after a skipped publish changed its side neighbour (`after_skip.contains(&border.revealed)`) |
| `cargo:mc-edge:anti_xray::anti_xray_tests::a_neighbour_unloaded_inside_a_skipped_publish_reveals_the_border_ore` | same, with the side neighbour unloaded (`None`) |
| `cargo:mc-edge:anti_xray::anti_xray_tests::a_neighbour_loaded_inside_a_skipped_publish_hides_the_border_ore` | the ore chunk is re-sent with the stand-in when the side loads |

`publish_sends_the_real_ore_when_a_block_beside_it_is_mined` (non-skipped publish) is green on both
trees and is not evidence.

## Why this is a real red

The tests call `v.skip_publication(&cache)` on the tree they run against, so the a/-side's one-line
method is the pre-image behaviour for the skip path: `skipped` is set, `skip_base` never exists. With
the `viewer_plan` hunks reversed there is no side repair at all, so a held chunk whose own revision did
not move is skipped and the client keeps the old (wrong) ore mask. Each test then fails at its own
`contains(...)` assertion — an observed assertion failure, not a compile error.

## Observed control and restore

Control `head` (dry-run `20261124T093710-ledger-promote-a11y0wla`, `ledger-1-run.log`), each ref red at
its own assertion (the triple-quoted macro starts on the named line, hence `proving_assertions`):

| ref | `file:line` | message |
|---|---|---|
| `…::a_border_reveal_inside_a_skipped_publish_reaches_the_client` | `anti_xray_tests.rs:480` | `the ore chunk is sent again with the real ore (1 chunk frames)` |
| `…::a_neighbour_unloaded_inside_a_skipped_publish_reveals_the_border_ore` | `anti_xray_tests.rs:496` | `the ore chunk is sent again with the real ore (0 chunk frames)` |
| `…::a_neighbour_loaded_inside_a_skipped_publish_hides_the_border_ore` | `anti_xray_tests.rs:515` | `the ore chunk is sent again with the stand-in (1 chunk frames)` |

The row's `proving_assertions` names those three locations. The restore is the real promotion receipt's
`ledger-3-run` on the candidate (all three green); see `promote.json` under
`.analysis/validation/<stamp>-ledger-promote-*/`.

## Not covered

The row's source notes a real client cannot discriminate this repair (the plain block-update path
delivers the held chunk's mask in both trees); `docs/development/real-client-gates.md` records that the
row keeps frame-level unit tests and a packet/frame-level witness is backlog. This promotion is that
frame-level witness.

## Conservative behaviour to keep in mind

With the chunk delta log disabled (`ChunkDeltaLog::enabled == false`), `previous()` returns `None`, so
`side_moved` reports every side as moved and, under anti-xray, a skip re-sends every held chunk —
conservative (no reveal is lost), just more frames. Recorded as a known, intended bound.
